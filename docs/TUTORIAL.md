# trapdoor tutorial — debugging bash like it's a real language

Fifteen minutes, four sessions, one hunted bug. Everything below is copy-pasteable;
the transcripts are real.

## 0. Setup

```sh
git clone https://github.com/Makeph/trapdoor
cd trapdoor
cargo build --release
alias trapdoor=$PWD/target/release/trapdoor   # or cargo install --path .
```

Requirements: bash ≥ 4.1 with `/dev/tcp` support (Linux, macOS, Git Bash on Windows
all qualify). Check yours:

```sh
bash -c 'exec 3<>/dev/tcp/1.1.1.1/80 && echo your bash can do this'
```

## 1. First contact: step through a script

```console
$ trapdoor examples/demo.sh

examples/demo.sh:3 [depth 1]
  → set -u
(tdb)
```

trapdoor stopped **before the first command ran**. The `→` line is the command bash
is about to execute; nothing has happened yet. You're in the `(tdb)` REPL:

- `s` (or just Enter) — step to the next command, following calls into functions
- `l` — show the source around the current line, with a `▶` marker
- `h` — the full command list

Press `s` a few times. When the script enters `greet()`, the depth indicator climbs
to `[depth 2]` — you're inside a function call. Try:

```console
(tdb) bt
#0  greet()  at examples/demo.sh:7
#1  main()  at examples/demo.sh:14
(tdb) p who msg
declare -- who="apple"
declare -- msg="hello, apple"
```

`p` runs `declare -p` **inside the live script** — arrays and associative maps
print properly. `f` (finish) runs the rest of the function and stops back in the
caller. `n` (next) steps *over* calls instead of into them. `c` continues freely.

## 2. Breakpoints, including conditional ones

Stopping at every line gets old. Set breakpoints from the command line:

```console
$ trapdoor -r -b 13 examples/demo.sh
```

`-b 13` breaks at line 13 of the main script; `-r` skips the initial stop and runs
straight to the first breakpoint. Inside the REPL, `b utils.sh:40` targets another
sourced file, `bl` lists, `d 1` deletes.

The good part — **conditions are raw bash, evaluated in the script**:

```console
$ trapdoor -r -b 'demo.sh:13 if (( count == 1 ))' examples/demo.sh
breakpoint #1 at demo.sh:13  if (( count == 1 ))
hello, apple

examples/demo.sh:13 [depth 1] ● breakpoint #1
  → count=$((count + 1))
(tdb) p count fruit
declare -- count="1"
declare -- fruit="banana"
```

It skipped the first loop iteration and stopped exactly on the second. Any bash
test works: `(( … ))`, `[[ $fruit == banana ]]`, even `grep -q err /tmp/log`.

## 3. Watches and `until`

Watches are expressions re-evaluated in the script and printed at **every stop**:

```console
$ trapdoor examples/demo.sh
(tdb) w $count
watch #1: $count
(tdb) w $fruit
watch #2: $fruit
(tdb) u 17
hello, apple
hello, banana
hello, cherry

examples/demo.sh:17 [depth 1]
  → total=$((count * 14))
  watch #1: $count = 3
  watch #2: $fruit = cherry
```

`u 17` means *run until line 17 of the current file* — a one-shot target that
disarms itself once it fires. The whole loop executed in one keystroke, and the
watches summarize the state on arrival. `wl` lists watches, `wd 1` deletes one.

## 4. The hunt: fix a live script without editing it

[examples/pipeline.sh](../examples/pipeline.sh) crunches web-server logs and
prints an average of 776 bytes per request. The true average is 679. Hunt:

```console
$ trapdoor -r -b 36 examples/pipeline.sh

examples/pipeline.sh:36 [depth 1] ● breakpoint #1
  → average=$((total_bytes / (requests - 1)))
(tdb) p requests errors total_bytes
declare -- requests="8"
declare -- errors="4"
declare -- total_bytes="5435"
(tdb) x echo "true avg: $(( total_bytes / requests ))"
true avg: 679
```

The counters are right; the divisor is wrong — `requests - 1` is an off-by-one
(there is no header line to skip). Now the killer feature: `x` and `!` run bash
**in the script's own execution context**, so assignments stick. Patch the
running process instead of the file:

```console
(tdb) !total_bytes=$(( 679 * (requests - 1) ))     # counter the bad divisor
(tdb) c
requests=8 errors=4 total_bytes=4753 avg_bytes=679
```

The script printed the correct average without a single edit to its source. In
real life you'd fix line 36 — but when the bug only reproduces forty minutes
into a batch run, rewriting a variable beats restarting.

## 5. Cheat sheet

| | |
|---|---|
| motion | `s` step into · `n` step over · `f` finish · `c` continue · `u <line>` until · Enter repeats |
| breakpoints | `b <line>` · `b <file>:<line> [if <raw-bash>]` · `bl` · `d <id>` |
| watches | `w <expr>` · `wl` · `wd <id>` |
| inspect | `p <var>…` · `bt` · `l [line]` |
| mutate | `x <code>` or `!<code>` — assignments persist |
| exit | `q` kills the script · Ctrl-D detaches and lets it run free |

## Notes for real-world scripts

- Commands inside `$( … )` and pipeline segments run in subshells; trapdoor stays
  quiet there and stops on the enclosing command instead.
- If your script reads stdin, it competes with the REPL — give one of them a
  redirect.
- The stub adds one TCP round-trip per command; fine for scripts, wrong tool for
  a hot loop doing a million iterations.
