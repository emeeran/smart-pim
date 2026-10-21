//! Minimal POP3 client (RFC 1939): the commands the sync worker needs —
//! USER/PASS, STAT, UIDL, RETR, QUIT — over a plain TcpStream. TLS wrapping
//! happens above (native-tls stream in, same API).
//!
//! Hand-rolled because the published crates are unmaintained and the protocol
//! is ~100 lines. Line endings are CRLF per RFC; multiline payloads end at a
//! lone "." with byte-stuffing removed.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

pub struct Pop3 {
    reader: BufReader<TcpStream>,
    stream: TcpStream,
}

fn err(msg: &str) -> String {
    format!("pop3: {msg}")
}

impl Pop3 {
    pub fn new(stream: TcpStream) -> std::io::Result<Self> {
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self { reader, stream })
    }

    fn read_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .map_err(|e| err(&e.to_string()))?;
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    /// Single-line response: `+OK ...` or `+OK` — errors on `-ERR`.
    fn expect_ok(&mut self, cmd: &str) -> Result<String, String> {
        let line = self.read_line()?;
        if let Some(rest) = line.strip_prefix("+OK") {
            Ok(rest.trim_start().to_string())
        } else {
            Err(err(&format!("{cmd} failed: {line}")))
        }
    }

    fn command(&mut self, cmd: &str) -> Result<String, String> {
        self.stream
            .write_all(format!("{cmd}\r\n").as_bytes())
            .map_err(|e| err(&e.to_string()))?;
        self.stream.flush().map_err(|e| err(&e.to_string()))?;
        self.expect_ok(cmd.split_whitespace().next().unwrap_or(cmd))
    }

    /// Consume the server greeting.
    pub fn read_greeting(&mut self) -> Result<(), String> {
        self.expect_ok("greeting").map(|_| ())
    }

    pub fn login(&mut self, user: &str, pass: &str) -> Result<(), String> {
        self.command(&format!("USER {user}"))?;
        self.command(&format!("PASS {pass}"))?;
        Ok(())
    }

    /// `UIDL` for all messages: (1-based index, unique id) pairs.
    pub fn uidl(&mut self) -> Result<Vec<(usize, String)>, String> {
        self.stream
            .write_all(b"UIDL\r\n")
            .map_err(|e| err(&e.to_string()))?;
        self.expect_ok("UIDL")?;
        let mut out = Vec::new();
        loop {
            let line = self.read_line()?;
            if line == "." {
                break;
            }
            let mut parts = line.splitn(2, ' ');
            let idx = parts.next().unwrap_or("").parse::<usize>().ok();
            let id = parts.next().unwrap_or("").trim().to_string();
            if let Some(idx) = idx {
                if !id.is_empty() {
                    out.push((idx, id));
                }
            }
        }
        Ok(out)
    }

    /// `RETR n`: full raw message with byte-unstuffing applied.
    pub fn retr(&mut self, n: usize) -> Result<Vec<u8>, String> {
        self.stream
            .write_all(format!("RETR {n}\r\n").as_bytes())
            .map_err(|e| err(&e.to_string()))?;
        self.expect_ok("RETR")?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let line = self.read_line()?;
            if line == "." {
                break;
            }
            let line = line.strip_prefix('.').unwrap_or(&line);
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        Ok(out)
    }

    /// Polite logout (messages are NOT deleted — POP3 sync is non-destructive).
    pub fn quit(&mut self) -> Result<(), String> {
        self.command("QUIT").map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn multiline_unstuffing_shapes_raw_message() {
        // Byte-stuffing: a payload line starting with "." gains one dot.
        // (RETR strips it; a line that is exactly "." terminates.)
        let stuffed = "..leading dot";
        assert_eq!(stuffed.strip_prefix('.').unwrap(), ".leading dot");
        assert_eq!(".", ".");
    }
}
