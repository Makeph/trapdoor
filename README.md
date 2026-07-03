# trapdoor

**An interactive step-debugger for Bash scripts — built entirely out of things bash already ships.**

No patched bash. No `ptrace`. No `set -x` archaeology. The debugger side of trapdoor is
~60 lines of pure bash injected through `BASH_ENV`; it talks to a Rust controller over
`/dev/tcp`, bash's built-in TCP socket. The `DEBUG` trap does the rest: every command in
your script pauses and asks permission before it runs.

Born from a wish on [Ask HN: What developer tool do you wish existed in 2026?](https://news.ycombinator.com/item?id=46345827):

> *"No debugging interface for shell scripts — stop at a specific point in the script,
> modify any commands and execute the step."*

So: breakpoints (conditional ones too), `step` / `next` / `finish`, backtraces, source
listings — and a REPL that evaluates **inside the live script**, so assignments stick.
Change a variable mid-loop and watch the script take the other branch.

## Demo

```console
$ trapdoor -b 'demo.sh:13 if (( count == 1 ))' -r examples/demo.sh
breakpoint #1 at demo.sh:13  if (( count == 1 ))
hello, apple

examples/demo.sh:13 [depth 1] ● breakpoint #1
  → count=$((count + 1))
(tdb) p count fruit
declare -- count="1"
declare -- fruit="banana"
(tdb) !count=40
(tdb) c
hello, banana
hello, cherry
processed 43 fruits, total=602
```

That `43` is not a typo — `!count=40` rewrote the loop counter in the running script.

## How it works

```
┌─────────────────────┐   TCP 127.0.0.1:<random>   ┌──────────────────────────┐
│ trapdoor (Rust)     │◄───────────────────────────►│ bash your-script.sh      │
│  breakpoints, REPL, │   STOP file:line:cmd  ───►  │  stub via BASH_ENV:      │
│  step logic, source │   ◄─── GO / EVAL / BT       │   exec {fd}<>/dev/tcp/…  │
│  listings, colors   │                             │   trap '…' DEBUG         │
└─────────────────────┘                             └──────────────────────────┘
```

1. The controller binds a random localhost port and launches your script with
   `BASH_ENV` pointing at a tiny stub ([src/stub.sh](src/stub.sh)).
2. The stub opens a socket with `exec {fd}<>/dev/tcp/127.0.0.1/$PORT` — no `nc`,
   no `socat`, no python; `/dev/tcp` is interpreted by bash itself.
3. It arms the `DEBUG` trap (with `set -o functrace` so functions inherit it).
   Before **every simple command**, the stub reports `file:line`, call depth and the
   command text, then blocks until the controller answers.
4. `GO` runs the command. `EVAL <code>` runs arbitrary bash **in the script's own
   execution context** — that's how `p`, `x`, `!` and conditional breakpoints work.
   `BT` walks `FUNCNAME`/`BASH_SOURCE`/`BASH_LINENO` for a backtrace.
5. If the controller dies, the stub disarms the trap and the script runs free.
   No zombie hostages.

## Install

```sh
cargo install --path .          # or: cargo build --release
```

One static-ish binary, zero crate dependencies (`std` only). Works anywhere bash is
compiled with `/dev/tcp` support — Linux distros, macOS, and Git Bash / MSYS2 on
Windows all qualify.

## Usage

```
trapdoor [OPTIONS] <script.sh> [script args...]

-b, --break <SPEC>   breakpoint: <line> | <file>:<line> | <file>:<line> if <bash-cond>
-r, --run            don't stop at the first command; run until a breakpoint
    --bash <PATH>    which bash to use
    --no-color       disable ANSI colors
```

At the `(tdb)` prompt:

| command | effect |
|---|---|
| `s` / `step` | stop at the next command, anywhere (steps into functions) |
| `n` / `next` | next command at this depth or shallower (steps over calls) |
| `f` / `finish` | run until the current function returns |
| `c` / `continue` | run until a breakpoint |
| `u <line>` | run until that line in the current file (one-shot) |
| *enter* | repeat the last motion command |
| `b 13` · `b utils.sh:40` | breakpoint |
| `b 13 if (( count == 2 ))` | conditional breakpoint — the condition is **raw bash** run in the script: `(( … ))`, `[[ … ]]`, even `grep -q …` |
| `bl` / `d <id>` | list / delete breakpoints |
| `w <bash-expr>` | watch: evaluated in the script and shown at every stop (`w $count`) |
| `wl` / `wd <id>` | list / delete watches |
| `p var…` | `declare -p` variables (arrays and maps print properly) |
| `x code` / `!code` | run bash in the live script — **assignments stick** |
| `bt` | backtrace |
| `l [line]` | source listing around the current (or given) line |
| `q` | kill the script and quit |

## Honest caveats

- Commands inside `$( … )` command substitutions and pipeline segments run in
  subshells; trapdoor deliberately stays quiet there (two writers on one socket
  would corrupt the protocol). You still stop on the enclosing command.
- The `DEBUG` trap fires per *simple command*, so a compound like `for …` stops
  once at the loop head, then at each body command — same as `bash -x` granularity.
- Scripts that read stdin share it with the debugger REPL. Redirect one of them.
- Requires bash ≥ 4.1 (for `{fd}<>` auto-allocation) built with `/dev/tcp`.

## Why not bashdb?

[bashdb](https://bashdb.sourceforge.net/) is venerable and more featureful, but it's a
1MB bash-in-bash interpreter you have to install on the target machine. trapdoor's
target-side footprint is one temp file and one socket, injected by the environment —
nothing to install where the script runs, and the brains stay in one fast binary.

## License

MIT
