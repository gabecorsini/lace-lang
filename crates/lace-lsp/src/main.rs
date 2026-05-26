mod document;
mod handler;

use std::io::{BufRead, Read, Write};

use handler::LspServer;

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut server = LspServer::new(stdout);

    let mut stdin_lock = stdin.lock();
    loop {
        // Read headers until blank line
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            match stdin_lock.read_line(&mut line) {
                Ok(0) => return, // EOF
                Ok(_) => {}
                Err(_) => return,
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            if let Some(rest) = line.strip_prefix("Content-Length: ") {
                content_length = rest.trim().parse().ok();
            }
        }

        let len = match content_length {
            Some(l) => l,
            None => continue,
        };

        let mut body = vec![0u8; len];
        if stdin_lock.read_exact(&mut body).is_err() {
            return;
        }

        let msg: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => continue,
        };

        server.handle(msg);
    }
}
