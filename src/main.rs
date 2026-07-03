//! trapdoor — an interactive step-debugger for Bash scripts.
//!
//! The controller (this program) opens a TCP listener on localhost, then
//! launches the target script with `BASH_ENV` pointing at a small pure-bash
//! stub. The stub connects back over `/dev/tcp` and arms the `DEBUG` trap,
//! so every simple command in the script pauses and asks the controller for
//! a verdict. Nothing is patched, traced or forked: the debugger is made
//! entirely out of things bash already ships.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{exit, Child, Command};
use std::thread;
use std::time::Duration;

const STUB: &str = include_str!("stub.sh");
const SENTINEL: &str = "\u{4}END";

// ---------------------------------------------------------------- options

struct Opts {
    script: String,
    script_args: Vec<String>,
    breaks: Vec<String>,
    run: bool,
    bash: String,
    color: bool,
}

fn usage(code: i32) -> ! {
    println!(
        "trapdoor {} — an interactive step-debugger for Bash scripts

USAGE:
    trapdoor [OPTIONS] <script.sh> [script args...]

OPTIONS:
    -b, --break <SPEC>   set a breakpoint before starting (repeatable)
                         SPEC:  <line> | <file>:<line> | <file>:<line> if <bash-cond>
                         the condition is raw bash, e.g.  '13 if (( count == 2 ))'
    -r, --run            don't stop at the first command; run until a breakpoint
        --bash <PATH>    bash executable to use (default: bash)
        --no-color       disable ANSI colors
    -h, --help           show this help
    -V, --version        show version

EXAMPLE:
    trapdoor -b 14 -b 'demo.sh:13 if (( count == 2 ))' examples/demo.sh

Type 'h' at the (tdb) prompt for debugger commands.",
        env!("CARGO_PKG_VERSION")
    );
    exit(code);
}

fn parse_args() -> Opts {
    let mut args = env::args().skip(1);
    let mut opts = Opts {
        script: String::new(),
        script_args: Vec::new(),
        breaks: Vec::new(),
        run: false,
        bash: "bash".into(),
        color: io::stdout().is_terminal(),
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => usage(0),
            "-V" | "--version" => {
                println!("trapdoor {}", env!("CARGO_PKG_VERSION"));
                exit(0);
            }
            "-b" | "--break" => match args.next() {
                Some(s) => opts.breaks.push(s),
                None => die("option -b requires an argument"),
            },
            "--bash" => match args.next() {
                Some(s) => opts.bash = s,
                None => die("option --bash requires an argument"),
            },
            "-r" | "--run" => opts.run = true,
            "--no-color" => opts.color = false,
            _ => {
                opts.script = a;
                opts.script_args = args.collect();
                break;
            }
        }
    }
    if opts.script.is_empty() {
        usage(2);
    }
    opts
}

fn die(msg: &str) -> ! {
    eprintln!("trapdoor: {msg}");
    exit(2);
}

// ------------------------------------------------------------------ model

#[derive(Clone, Copy, PartialEq)]
enum RunMode {
    Step,
    Next(u32),
    Finish(u32),
    Continue,
}

struct Breakpoint {
    id: u32,
    file: String,
    line: u32,
    cond: Option<String>,
    hits: u64,
}

struct Pal {
    dim: &'static str,
    bold: &'static str,
    red: &'static str,
    green: &'static str,
    yellow: &'static str,
    cyan: &'static str,
    reset: &'static str,
}

impl Pal {
    fn new(on: bool) -> Self {
        if on {
            Pal {
                dim: "\x1b[2m",
                bold: "\x1b[1m",
                red: "\x1b[31m",
                green: "\x1b[32m",
                yellow: "\x1b[33m",
                cyan: "\x1b[36m",
                reset: "\x1b[0m",
            }
        } else {
            Pal { dim: "", bold: "", red: "", green: "", yellow: "", cyan: "", reset: "" }
        }
    }
}

struct Session {
    tx: TcpStream,
    rx: BufReader<TcpStream>,
    pal: Pal,
    bps: Vec<Breakpoint>,
    next_bp_id: u32,
    mode: RunMode,
    last_motion: String,
    detached: bool,
    src_cache: HashMap<String, Option<Vec<String>>>,
    script: String,
}

enum ReplOutcome {
    Resume,
    Quit,
}

impl Session {
    fn send(&mut self, msg: &str) -> io::Result<()> {
        writeln!(self.tx, "{msg}")
    }

    fn recv(&mut self) -> Option<String> {
        let mut buf = String::new();
        match self.rx.read_line(&mut buf) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                while buf.ends_with('\n') || buf.ends_with('\r') {
                    buf.pop();
                }
                Some(buf)
            }
        }
    }

    /// Read stub output until the end-of-frame sentinel; returns (output, status).
    fn read_framed(&mut self) -> (String, i32) {
        let mut out = String::new();
        loop {
            match self.recv() {
                None => return (out, -1),
                Some(line) => {
                    if let Some(rest) = line.strip_prefix(SENTINEL) {
                        // The stub always emits one extra '\n' before the
                        // sentinel; fold it back out.
                        if out.ends_with('\n') {
                            out.pop();
                        }
                        return (out, rest.trim().parse().unwrap_or(-1));
                    }
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
    }

    /// Evaluate bash code inside the live script and return its output.
    fn remote_eval(&mut self, code: &str) -> (String, i32) {
        let one_line = code.replace('\n', "; ");
        if self.send(&format!("EVAL {one_line}")).is_err() {
            return (String::new(), -1);
        }
        self.read_framed()
    }

    fn backtrace(&mut self) -> String {
        if self.send("BT").is_err() {
            return String::new();
        }
        self.read_framed().0
    }

    // ------------------------------------------------------- breakpoints

    fn add_breakpoint(&mut self, spec: &str) {
        let (loc, cond) = match spec.split_once(" if ") {
            Some((l, c)) => (l.trim(), Some(c.trim().to_string())),
            None => (spec.trim(), None),
        };
        let (file, line) = match loc.rsplit_once(':') {
            Some((f, l)) => (f.to_string(), l.parse::<u32>()),
            None => (self.script.clone(), loc.parse::<u32>()),
        };
        let Ok(line) = line else {
            println!("{}bad breakpoint spec: {spec}{}", self.pal.red, self.pal.reset);
            println!("expected: <line> | <file>:<line> | <file>:<line> if <bash-cond>");
            return;
        };
        let id = self.next_bp_id;
        self.next_bp_id += 1;
        let cond_str = cond
            .as_deref()
            .map(|c| format!("  if {c}"))
            .unwrap_or_default();
        println!(
            "{}breakpoint #{id}{} at {}:{}{}",
            self.pal.green, self.pal.reset, basename(&file), line, cond_str
        );
        self.bps.push(Breakpoint { id, file, line, cond, hits: 0 });
    }

    /// Does any breakpoint fire at src:line? Conditions run in the script.
    fn breakpoint_hit(&mut self, src: &str, line: u32) -> Option<u32> {
        let mut candidate: Option<usize> = None;
        for (i, bp) in self.bps.iter().enumerate() {
            if bp.line == line && paths_match(&bp.file, src) {
                candidate = Some(i);
                break;
            }
        }
        let i = candidate?;
        if let Some(cond) = self.bps[i].cond.clone() {
            // The condition is raw bash run in the live script: use whatever
            // test fits — (( count == 2 )), [[ $fruit == banana ]], grep -q …
            let (_, status) = self.remote_eval(&cond);
            if status != 0 {
                return None;
            }
        }
        self.bps[i].hits += 1;
        Some(self.bps[i].id)
    }

    fn list_breakpoints(&self) {
        if self.bps.is_empty() {
            println!("no breakpoints");
            return;
        }
        for bp in &self.bps {
            let cond = bp
                .cond
                .as_deref()
                .map(|c| format!("  if {c}"))
                .unwrap_or_default();
            println!(
                "  #{}  {}:{}{}  {}({} hits){}",
                bp.id, bp.file, bp.line, cond, self.pal.dim, bp.hits, self.pal.reset
            );
        }
    }

    // ------------------------------------------------------------ source

    fn source_lines(&mut self, file: &str) -> Option<&Vec<String>> {
        self.src_cache
            .entry(file.to_string())
            .or_insert_with(|| {
                fs::read_to_string(file)
                    .ok()
                    .map(|s| s.lines().map(String::from).collect())
            })
            .as_ref()
    }

    fn list_source(&mut self, file: &str, center: u32, cur: u32) {
        let pal_dim = self.pal.dim;
        let pal_yellow = self.pal.yellow;
        let pal_reset = self.pal.reset;
        let Some(lines) = self.source_lines(file) else {
            println!("{pal_dim}(source not readable: {file}){pal_reset}");
            return;
        };
        let total = lines.len() as u32;
        let from = center.saturating_sub(4).max(1);
        let to = (center + 4).min(total);
        for n in from..=to {
            let text = &lines[(n - 1) as usize];
            if n == cur {
                println!("{pal_yellow}{n:>5} ▶ {text}{pal_reset}");
            } else {
                println!("{pal_dim}{n:>5}{pal_reset}   {text}");
            }
        }
    }

    // -------------------------------------------------------------- repl

    fn announce(&mut self, src: &str, line: u32, depth: u32, cmd: &str, bp: Option<u32>) {
        let tag = match bp {
            Some(id) => format!(
                " {}● breakpoint #{id}{}",
                self.pal.red, self.pal.reset
            ),
            None => String::new(),
        };
        println!(
            "\n{}{}:{}{} {}[depth {}]{}{}",
            self.pal.bold, src, line, self.pal.reset, self.pal.dim, depth, self.pal.reset, tag
        );
        println!("  {}→ {}{}", self.pal.cyan, cmd, self.pal.reset);
    }

    fn repl(&mut self, src: &str, line: u32, depth: u32, child: &mut Child) -> ReplOutcome {
        loop {
            print!("{}(tdb){} ", self.pal.green, self.pal.reset);
            let _ = io::stdout().flush();

            let mut input = String::new();
            let n = io::stdin().read_line(&mut input).unwrap_or(0);
            if n == 0 {
                // stdin closed: detach and let the script finish on its own.
                println!("{}stdin closed — detaching, script continues{}", self.pal.dim, self.pal.reset);
                self.detached = true;
                let _ = self.send("GO");
                return ReplOutcome::Resume;
            }

            let input = input.trim().to_string();
            let input = if input.is_empty() { self.last_motion.clone() } else { input };
            let (verb, rest) = match input.split_once(char::is_whitespace) {
                Some((v, r)) => (v, r.trim().to_string()),
                None => (input.as_str(), String::new()),
            };

            match verb {
                "s" | "step" => {
                    self.mode = RunMode::Step;
                    self.last_motion = "s".into();
                    let _ = self.send("GO");
                    return ReplOutcome::Resume;
                }
                "n" | "next" => {
                    self.mode = RunMode::Next(depth);
                    self.last_motion = "n".into();
                    let _ = self.send("GO");
                    return ReplOutcome::Resume;
                }
                "f" | "finish" => {
                    self.mode = RunMode::Finish(depth);
                    self.last_motion = "f".into();
                    let _ = self.send("GO");
                    return ReplOutcome::Resume;
                }
                "c" | "continue" => {
                    self.mode = RunMode::Continue;
                    self.last_motion = "c".into();
                    let _ = self.send("GO");
                    return ReplOutcome::Resume;
                }
                "q" | "quit" => {
                    let _ = self.send("KILL");
                    let _ = child.kill();
                    return ReplOutcome::Quit;
                }
                "b" | "break" => {
                    if rest.is_empty() {
                        println!("usage: b <line> | b <file>:<line> [if <bash-cond>]");
                    } else {
                        self.add_breakpoint(&rest);
                    }
                }
                "bl" | "breaks" => self.list_breakpoints(),
                "d" | "delete" => match rest.parse::<u32>() {
                    Ok(id) => {
                        let before = self.bps.len();
                        self.bps.retain(|b| b.id != id);
                        if self.bps.len() == before {
                            println!("no breakpoint #{id}");
                        } else {
                            println!("deleted breakpoint #{id}");
                        }
                    }
                    Err(_) => println!("usage: d <breakpoint-id>"),
                },
                "p" | "print" => {
                    if rest.is_empty() {
                        println!("usage: p <var> [var...]");
                    } else {
                        for name in rest.split_whitespace() {
                            let code = format!(
                                "declare -p {name} 2>/dev/null || printf '%s: not set\\n' {name}"
                            );
                            let (out, _) = self.remote_eval(&code);
                            print!("{out}");
                        }
                    }
                }
                "x" | "eval" | "!" => {
                    if rest.is_empty() {
                        println!("usage: x <bash code>   (runs inside the live script)");
                    } else {
                        let (out, status) = self.remote_eval(&rest);
                        print!("{out}");
                        if status != 0 {
                            println!("{}(exit status {status}){}", self.pal.dim, self.pal.reset);
                        }
                    }
                }
                "bt" | "where" => {
                    let out = self.backtrace();
                    if out.is_empty() {
                        println!("(top level)");
                    } else {
                        print!("{out}");
                    }
                }
                "l" | "list" => {
                    let center = rest.parse::<u32>().unwrap_or(line);
                    self.list_source(src, center, line);
                }
                "h" | "help" | "?" => print_repl_help(),
                other => {
                    // A leading '!' glued to code: `!count=99`
                    if let Some(code) = other.strip_prefix('!') {
                        let full = format!("{code} {rest}");
                        let (out, _) = self.remote_eval(full.trim());
                        print!("{out}");
                    } else {
                        println!("unknown command '{other}' — try 'h'");
                    }
                }
            }
        }
    }
}

fn print_repl_help() {
    println!(
        "motion:
  s, step        stop at the next command, anywhere (steps into functions)
  n, next        stop at the next command at this depth or shallower
  f, finish      run until the current function returns
  c, continue    run until a breakpoint
  <enter>        repeat the last motion command
breakpoints:
  b <line>                        break in the main script
  b <file>:<line> [if <cond>]     cond is raw bash, e.g.  b 13 if (( count == 2 ))
  bl                              list breakpoints        d <id>   delete one
inspect / mutate (all run inside the live script):
  p <var>...     declare -p a variable (arrays too)
  x <code>       run any bash code — assignments stick:  x count=99
  !<code>        same, shorthand:  !name=world
  bt             backtrace          l [line]   show source
  q              kill the script and quit"
    );
}

// ---------------------------------------------------------------- helpers

fn basename(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

/// Loose path equality: exact match, or same basename when either side
/// carries no directory. `./demo.sh` and `demo.sh` should agree.
fn paths_match(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim_start_matches("./").to_string();
    let (a, b) = (norm(a), norm(b));
    a == b || basename(&a) == basename(&b)
}

// ------------------------------------------------------------------- main

fn main() {
    let opts = parse_args();
    let pal = Pal::new(opts.color);

    if !std::path::Path::new(&opts.script).exists() {
        die(&format!("script not found: {}", opts.script));
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|e| die(&format!("cannot bind localhost listener: {e}")));
    let port = listener.local_addr().unwrap().port();

    // The stub travels as a temp file so BASH_ENV can point at it.
    let stub_path = env::temp_dir().join(format!("trapdoor-stub-{}.sh", std::process::id()));
    fs::write(&stub_path, STUB)
        .unwrap_or_else(|e| die(&format!("cannot write stub: {e}")));
    let stub_env = stub_path.to_string_lossy().replace('\\', "/");

    let mut child = Command::new(&opts.bash)
        .arg(&opts.script)
        .args(&opts.script_args)
        .env("BASH_ENV", &stub_env)
        .env("TRAPDOOR_PORT", port.to_string())
        .spawn()
        .unwrap_or_else(|e| die(&format!("cannot launch {}: {e}", opts.bash)));

    eprintln!(
        "{}trapdoor {} — controlling `{} {}` over 127.0.0.1:{} (h for help){}",
        pal.dim,
        env!("CARGO_PKG_VERSION"),
        opts.bash,
        opts.script,
        port,
        pal.reset
    );

    // Wait for the stub to phone home, but notice if the script dies first
    // (e.g. a bash built without /dev/tcp support).
    listener.set_nonblocking(true).ok();
    let stream = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                if let Ok(Some(status)) = child.try_wait() {
                    let _ = fs::remove_file(&stub_path);
                    eprintln!(
                        "trapdoor: script exited (status {:?}) before the stub connected — \
                         is this bash built with /dev/tcp support?",
                        status.code()
                    );
                    exit(status.code().unwrap_or(1));
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => die(&format!("accept failed: {e}")),
        }
    };
    stream.set_nonblocking(false).ok();
    stream.set_nodelay(true).ok();

    let rx = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut sess = Session {
        tx: stream,
        rx,
        pal,
        bps: Vec::new(),
        next_bp_id: 1,
        mode: if opts.run { RunMode::Continue } else { RunMode::Step },
        last_motion: "s".into(),
        detached: false,
        src_cache: HashMap::new(),
        script: opts.script.clone(),
    };
    for spec in &opts.breaks {
        sess.add_breakpoint(spec);
    }

    let stub_name = format!("trapdoor-stub-{}.sh", std::process::id());

    // -------------------------------------------------- main verdict loop
    let mut quit = false;
    while let Some(msg) = sess.recv() {
        let Some(rest) = msg.strip_prefix("STOP\t") else { continue };
        let mut parts = rest.splitn(4, '\t');
        let src = parts.next().unwrap_or("?").to_string();
        let line: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let depth: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
        let cmd = parts.next().unwrap_or("").to_string();

        if sess.detached || src.ends_with(&stub_name) {
            let _ = sess.send("GO");
            continue;
        }

        let bp = sess.breakpoint_hit(&src, line);
        let stop = bp.is_some()
            || match sess.mode {
                RunMode::Step => true,
                RunMode::Next(d) => depth <= d,
                RunMode::Finish(d) => depth < d,
                RunMode::Continue => false,
            };
        if !stop {
            let _ = sess.send("GO");
            continue;
        }

        sess.announce(&src, line, depth, &cmd, bp);
        match sess.repl(&src, line, depth, &mut child) {
            ReplOutcome::Resume => {}
            ReplOutcome::Quit => {
                quit = true;
                break;
            }
        }
    }

    let status = child.wait().ok();
    let _ = fs::remove_file(&stub_path);
    if quit {
        eprintln!("{}trapdoor: script killed{}", sess.pal.dim, sess.pal.reset);
        exit(130);
    }
    let code = status.and_then(|s| s.code()).unwrap_or(0);
    eprintln!(
        "\n{}trapdoor: script exited with status {}{}",
        sess.pal.dim, code, sess.pal.reset
    );
    exit(code);
}
