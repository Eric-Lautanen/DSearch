use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

/// Background API fetcher — runs API calls on a worker thread so the
/// egui UI never blocks on network I/O.
///
/// The UI thread calls `ensure_requested` which only fires a request
/// if one isn't already pending and the minimum interval has elapsed.
/// A dedicated worker thread performs the blocking HTTP call and
/// sends the response back. The UI polls for completed responses
/// each frame via `poll`.
pub struct AsyncApi {
    req_tx: mpsc::Sender<ApiRequest>,
    resp_rx: mpsc::Receiver<ApiResponse>,
    pending: std::collections::HashSet<String>,
    last_request: std::collections::HashMap<String, Instant>,
    min_interval: std::time::Duration,
}

#[derive(Debug)]
struct ApiRequest {
    path: String,
    data_dir: PathBuf,
}

#[derive(Debug)]
pub struct ApiResponse {
    pub path: String,
    pub body: Option<String>,
}

impl AsyncApi {
    pub fn new(_data_dir: PathBuf) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<ApiRequest>();
        let (resp_tx, resp_rx) = mpsc::channel::<ApiResponse>();

        std::thread::spawn(move || {
            while let Ok(req) = req_rx.recv() {
                let body = crate::cli::api_client::api_get_from_dir(&req.data_dir, &req.path);
                let resp = ApiResponse {
                    path: req.path,
                    body,
                };
                if resp_tx.send(resp).is_err() {
                    break;
                }
            }
        });

        Self {
            req_tx,
            resp_rx,
            pending: std::collections::HashSet::new(),
            last_request: std::collections::HashMap::new(),
            min_interval: std::time::Duration::from_secs(2),
        }
    }

    /// Request an API call only if one isn't already pending and
    /// the minimum interval since the last request for this path
    /// has elapsed.
    pub fn ensure_requested(&mut self, path: &str, data_dir: &Path) {
        if self.pending.contains(path) {
            return;
        }
        if let Some(last) = self.last_request.get(path) {
            if last.elapsed() < self.min_interval {
                return;
            }
        }
        let req = ApiRequest {
            path: path.to_string(),
            data_dir: data_dir.to_path_buf(),
        };
        if self.req_tx.send(req).is_ok() {
            self.pending.insert(path.to_string());
            self.last_request.insert(path.to_string(), Instant::now());
        }
    }

    /// Poll for completed responses. Returns (path, body) pairs.
    pub fn poll(&mut self) -> Vec<(String, Option<String>)> {
        let mut results = Vec::new();
        loop {
            match self.resp_rx.try_recv() {
                Ok(resp) => {
                    self.pending.remove(&resp.path);
                    results.push((resp.path, resp.body));
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        results
    }
}
