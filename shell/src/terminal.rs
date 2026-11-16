//! Text from apps and platform responses must remain text in a terminal.

use std::fmt::{self, Write as _};
use std::io::{self, Write};
use std::sync::{OnceLock, mpsc};

/// Include escaping and app-id prefixes in the native output budget.
const MAX_BYTES: usize = 64 * 1024;
const TRUNCATED: &str = " [truncated]\n";
/// At most 4 MB of console text waits behind a stalled collector, plus its current message.
const QUEUED: usize = 64;

struct Text {
    value: String,
}

impl Text {
    fn new() -> Self {
        Self { value: String::new() }
    }

    fn finish(mut self, truncated: bool) -> String {
        if truncated {
            if self.value.ends_with('\n') {
                self.value.pop();
            }
            self.value.push_str(TRUNCATED);
        } else if !self.value.ends_with('\n') {
            self.value.push('\n');
        }
        self.value
    }
}

impl fmt::Write for Text {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for c in text.chars() {
            let control = c.is_control() && !matches!(c, '\n' | '\t');
            let bytes = if control { c.escape_default().count() } else { c.len_utf8() };
            if self.value.len() + bytes > MAX_BYTES - TRUNCATED.len() {
                return Err(fmt::Error);
            }
            if control {
                self.value.extend(c.escape_default());
            } else {
                self.value.push(c);
            }
        }
        Ok(())
    }
}

/// Native log destinations; console data printed by the CLI remains synchronous.
pub enum Stream {
    Stdout,
    Stderr,
}

struct Entry {
    stream: Stream,
    text: String,
}

fn enqueue(stream: Stream, text: String) {
    static WRITER: OnceLock<Option<mpsc::SyncSender<Entry>>> = OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Entry>(QUEUED);
        std::thread::Builder::new()
            .name("appos-console".into())
            .spawn(move || {
                for entry in rx {
                    match entry.stream {
                        Stream::Stdout => {
                            let _ = io::stdout().lock().write_all(entry.text.as_bytes());
                        }
                        Stream::Stderr => {
                            let _ = io::stderr().lock().write_all(entry.text.as_bytes());
                        }
                    }
                }
            })
            .ok()
            .map(|_| tx)
    });
    if let Some(writer) = writer {
        // A collector may close or stop reading. Neither may hold up a request.
        let _ = writer.try_send(Entry { stream, text });
    }
}

/// Native status and diagnostic logs are best effort, off the async workers.
pub fn native_log(stream: Stream, args: fmt::Arguments<'_>) {
    let mut text = Text::new();
    let truncated = text.write_fmt(args).is_err();
    enqueue(stream, text.finish(truncated));
}

/// Preserve readable Unicode, tabs and newlines; quote other control
/// characters so they cannot clear the screen, move the cursor or start OSC.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && !matches!(c, '\n' | '\t') {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

fn app_text<'a>(app_id: &str, lines: impl IntoIterator<Item = &'a str>) -> String {
    let prefix = format!("[{}] ", escape(app_id));
    let mut output = Text::new();
    let mut truncated = false;
    'messages: for text in lines {
        for line in text.split('\n') {
            // Never cut an app-id prefix in half when starting a new line.
            if output.value.len() + prefix.len() > MAX_BYTES - TRUNCATED.len() || writeln!(output, "{prefix}{line}").is_err() {
                truncated = true;
                break 'messages;
            }
        }
    }
    output.finish(truncated)
}

/// Retained error details stay complete; only the native console is bounded.
pub fn app_logs(app_id: &str, lines: &[String]) {
    if !lines.is_empty() {
        enqueue(Stream::Stderr, app_text(app_id, lines.iter().map(String::as_str)));
    }
}

/// Console data on stdout, with terminal controls escaped.
#[macro_export]
macro_rules! terminal_print {
    ($($arg:tt)*) => {
        ::std::print!("{}", $crate::terminal::escape(&::std::format!($($arg)*)))
    };
}

/// Console data on stdout, with terminal controls escaped, ending in a newline.
#[macro_export]
macro_rules! terminal_println {
    () => { ::std::println!() };
    ($($arg:tt)+) => {
        ::std::println!("{}", $crate::terminal::escape(&::std::format!($($arg)+)))
    };
}

/// Native shell messages on stdout. Log pipes may close independently of
/// requests, so logging failures must not abort an HTTP response.
#[macro_export]
macro_rules! terminal_logln {
    () => { $crate::terminal_logln!("") };
    ($($arg:tt)+) => {
        $crate::terminal::native_log($crate::terminal::Stream::Stdout, ::std::format_args!($($arg)+))
    };
}

/// Native diagnostics on stderr, through the same bounded logging worker.
#[macro_export]
macro_rules! terminal_errorln {
    () => { $crate::terminal_errorln!("") };
    ($($arg:tt)+) => {
        $crate::terminal::native_log($crate::terminal::Stream::Stderr, ::std::format_args!($($arg)+))
    };
}

/// Diagnostic messages on stderr: escaped and best effort.
#[macro_export]
macro_rules! terminal_eprintln {
    () => { $crate::terminal_eprintln!("") };
    ($($arg:tt)+) => {{
        use ::std::io::Write as _;
        let _ = ::std::writeln!(::std::io::stderr().lock(), "{}", $crate::terminal::escape(&::std::format!($($arg)+)));
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_commands_are_displayed_as_text() {
        let input = "before\x1b[2J\x1b]52;c;payload\x07\r\x08\x7f\u{9b}2J";
        assert_eq!(escape(input), r"before\u{1b}[2J\u{1b}]52;c;payload\u{7}\r\u{8}\u{7f}\u{9b}2J");
    }

    #[test]
    fn ordinary_unicode_and_formatting_remain_readable() {
        let input = "München\n\t東京 — مرحبا";
        assert_eq!(escape(input), input);
    }

    #[test]
    fn app_messages_cannot_forge_an_unprefixed_line() {
        let output = app_text("test", ["before\n[other] forged\r\x1b[2J\nlast"]);
        assert_eq!(output, "[test] before\n[test] [other] forged\\r\\u{1b}[2J\n[test] last\n");
    }

    #[test]
    fn native_log_limits_count_the_escaped_bytes() {
        let mut text = Text::new();
        let truncated = write!(text, "{}", "\x1b".repeat(64_000)).is_err();
        let output = text.finish(truncated);
        assert!(truncated);
        assert!(output.len() <= MAX_BYTES);
        assert!(!output.contains('\x1b'));
        assert!(output.ends_with("[truncated]\n"));
    }

    #[test]
    fn prefixes_and_escaping_cannot_amplify_output_past_64_kb() {
        let input = "\n".repeat(64_000);
        let output = app_text(&"a".repeat(255), [input.as_str()]);
        assert!(output.len() <= 64 * 1024, "prefixes amplified the log to {} bytes", output.len());
    }

    #[test]
    fn truncation_keeps_utf8_and_app_prefixes() {
        let input = "界\x1b[2J\n".repeat(20_000);
        let text = app_text("test", [input.as_str()]);
        assert!(text.len() <= 64 * 1024);
        assert!(!text.contains('\u{fffd}'));
        assert!(text.ends_with("[truncated]\n"));
        assert!(text.lines().all(|line| line.starts_with("[test] ")));
    }
}
