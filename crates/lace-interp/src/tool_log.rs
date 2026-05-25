use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct ToolLogger {
    pub run_id: String,
    pub suppress: bool,
    pub log_file: Option<std::fs::File>,
    pub count_calls: u32,
    pub count_ok: u32,
    pub count_err: u32,
}

impl ToolLogger {
    pub fn new(suppress: bool, log_file: Option<&str>) -> Self {
        let run_id = generate_run_id();
        let file = log_file.and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        });
        Self {
            run_id,
            suppress,
            log_file: file,
            count_calls: 0,
            count_ok: 0,
            count_err: 0,
        }
    }

    fn ts_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn emit(&mut self, line: &str) {
        if !self.suppress {
            eprintln!("{line}");
        }
        if let Some(ref mut f) = self.log_file {
            let _ = writeln!(f, "{line}");
        }
    }

    pub fn log_call(&mut self, tool: &str, args: &[String]) {
        self.count_calls += 1;
        let ts_ms = Self::ts_ms();
        let run_id = self.run_id.clone();
        // Truncate each arg to 100 chars
        let truncated: Vec<String> = args.iter().map(|a| {
            if a.len() > 100 { format!("{}...", &a[..97]) } else { a.clone() }
        }).collect();
        let args_json = serde_json::to_string(&truncated).unwrap_or_else(|_| "[]".into());
        let line = format!(
            r#"{{"event":"tool_call","tool":"{tool}","args":{args_json},"ts_ms":{ts_ms},"run_id":"{run_id}"}}"#
        );
        self.emit(&line);
    }

    pub fn log_ok(&mut self, tool: &str, duration_ms: u64) {
        self.count_ok += 1;
        let run_id = self.run_id.clone();
        let line = format!(
            r#"{{"event":"tool_ok","tool":"{tool}","result_type":"Ok","duration_ms":{duration_ms},"run_id":"{run_id}"}}"#
        );
        self.emit(&line);
    }

    pub fn log_err(&mut self, tool: &str, error: &str, duration_ms: u64) {
        self.count_err += 1;
        let run_id = self.run_id.clone();
        // Escape error string for JSON
        let error_escaped = error.replace('\\', "\\\\").replace('"', "\\\"");
        let line = format!(
            r#"{{"event":"tool_err","tool":"{tool}","error":"{error_escaped}","duration_ms":{duration_ms},"run_id":"{run_id}"}}"#
        );
        self.emit(&line);
    }

    pub fn log_retry(&mut self, tool: &str, attempt: u32, max: i64) {
        let run_id = self.run_id.clone();
        let line = format!(
            r#"{{"event":"tool_retry","tool":"{tool}","attempt":{attempt},"max":{max},"run_id":"{run_id}"}}"#
        );
        self.emit(&line);
    }

    pub fn log_timeout(&mut self, tool: &str, ms: i64) {
        let run_id = self.run_id.clone();
        let line = format!(
            r#"{{"event":"tool_timeout","tool":"{tool}","ms":{ms},"run_id":"{run_id}"}}"#
        );
        self.emit(&line);
    }

    /// Returns a summary line if any tool calls were made, otherwise None.
    pub fn summary(&self) -> Option<String> {
        if self.count_calls == 0 {
            return None;
        }
        Some(format!(
            "[lace] run_id={}  tools={}  ok={}  err={}",
            self.run_id, self.count_calls, self.count_ok, self.count_err
        ))
    }
}

fn generate_run_id() -> String {
    // Generate 8-char hex using system time + some xor for variety
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // mix bits
    let mixed = t ^ (t >> 17) ^ (t << 3).wrapping_mul(0xDEADBEEF);
    format!("{:08x}", mixed as u32)
}
