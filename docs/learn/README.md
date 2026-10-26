# Learn Mickey: a guided tour

Eight short chapters that take you from "what is this?" to "I can explain
every hop a key press makes, and why it is built that way". Each one is a
story, not a reference: it opens with a question, follows the real code
file by file, and ends with something to try and a few questions to check
yourself.

The reference documents stay where they are and are worth a look when a
chapter points at them: [architecture.md](../architecture.md) (crates,
threads, the platform layer), [protocol.md](../protocol.md) (every byte
on the wire), [flows.md](../flows.md) (pairing, focus, edges, the
switcher, the clipboard, with diagrams), the PRDs in [prds/](../prds/) and
the glossary in [CONTEXT.md](../../CONTEXT.md).

## How each chapter works

1. **The question.** Read it and guess the answer before going on. A wrong
   guess is the point: it shows you what you did not know yet.
2. **The path.** Pointers look like
   [`capture.rs:181`](../../crates/mickey/src/capture.rs#L181). Open the
   file at that line (in VS Code: Ctrl+P, type `capture.rs:181`) and read
   a screenful around it. Line numbers match commit `5676d2c` (version
   0.2.1); if the code moved, search for the function name next to the
   pointer.
3. **Try it.** One small thing to run or read. None of them grabs your
   keyboard or mouse, and none prints what you type.
4. **Check yourself.** 3 to 5 questions. Answer out loud, as if a friend
   asked, then open the collapsed answers.

## Reading order and time

| # | Chapter | Time | You will be able to explain |
|---|---|---|---|
| 1 | [What Mickey is and why it exists](01-what-and-why.md) | 20 min | The problem, who has it, the alternatives, what is different |
| 2 | [The journey of a key press](02-key-press.md) | 45 min | Capture, focus, the secure link, emit, on Linux and on Windows |
| 3 | [Pairing and trust](03-pairing-and-trust.md) | 40 min | Discovery, the six-digit code, `trusted_peers`, the group |
| 4 | [The journey of a copy](04-copy.md) | 35 min | The data link, `Ask`, images and files, the history |
| 5 | [The journey of a sound](05-sound.md) | 40 min | Divert, record, seal, UDP, the jitter buffer, play |
| 6 | [Safety nets](06-safety-nets.md) | 30 min | Every way home, signals, Windows admin mode, input never logged |
| 7 | [How it is built, tested and released](07-build-test-release.md) | 30 min | Crates, CI, `release.yml`, the installers |
| 8 | [The pitch](08-the-pitch.md) | 30 min | Mickey in 30 s, 2 min, 5 min; the commercial angle; interview answers |

About five hours in all. Chapters 1 and 2 first; after that any order
works, though 3 before 4 and 5 helps (both lean on the data link and on
trust). Chapter 8 is the payoff: do it last, out loud.

## Tips that make it stick

- **Draw it.** After chapter 2, draw the key press on paper from memory:
  boxes for threads, arrows for channels and sockets. Compare with the
  sequence diagram in [architecture.md](../architecture.md).
- **Break it on purpose.** In a scratch branch, change one constant a
  chapter mentions (say `SILENT_LIMIT`) and run `cargo test`. Which test
  fails tells you what the constant protects.
- **Use the debugger.** A breakpoint in a test (CodeLLDB, *Debug* above a
  `#[test]`) shows the values the chapter talks about.
- **Explain it to someone.** Chapter 8's questions are the ones a friend
  or an interviewer will ask. If an answer feels shaky, the chapter it
  came from is the one to read again.
