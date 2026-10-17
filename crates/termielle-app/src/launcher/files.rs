use super::model::{App, MAX_RESULTS, Target, document_score};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, ReadDir};
use std::io::{Read, Write};
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetLogicalDrives, MOVEFILE_REPLACE_EXISTING, MoveFileExW,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
};
use windows::core::PCWSTR;

const MAX_FILES: usize = 50_000;
const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_VISITED: usize = 1_000_000;
const MAX_HISTORY: usize = 512;
const SKIP_ATTRIBUTES: u32 = 0x2 | 0x4 | 0x400 | 0x1000 | 0x40000 | 0x400000;

pub(super) enum Command {
    Search(u64, String),
    Opened(PathBuf),
}

pub(super) struct Reply {
    pub generation: u64,
    pub entries: Vec<App>,
    pub status: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Usage {
    count: u32,
    last_opened: u64,
}

impl Usage {
    fn bonus(&self, now: u64) -> usize {
        let multiplier = match now.saturating_sub(self.last_opened) {
            0..=3_600 => 4,
            3_601..=86_400 => 2,
            86_401..=604_800 => 1,
            _ => 0,
        };
        (self.count.saturating_mul(multiplier).min(60)) as usize
    }
}

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    paths: Vec<PathBuf>,
    history: HashMap<PathBuf, Usage>,
    limited: bool,
}

struct Document {
    path: PathBuf,
    key: String,
}

#[derive(Default)]
struct Index {
    documents: Vec<Document>,
    bytes: usize,
    limited: bool,
    history: HashMap<PathBuf, Usage>,
}

impl Index {
    fn insert(&mut self, path: PathBuf) -> bool {
        let key = path.to_string_lossy().replace('/', "\\").to_lowercase();
        let bytes = key.len() * 2 + std::mem::size_of::<Document>();
        if self.documents.len() >= MAX_FILES || self.bytes + bytes > MAX_INDEX_BYTES {
            self.limited = true;
            return false;
        }
        self.bytes += bytes;
        self.documents.push(Document { path, key });
        true
    }

    fn search(&self, query: &str, now: u64) -> Vec<App> {
        let query = query.to_lowercase().replace('/', "\\");
        let tokens: Vec<_> = query.split_whitespace().collect();
        if tokens.is_empty() {
            return Vec::new();
        }
        let mut best = Vec::with_capacity(MAX_RESULTS + 1);
        for (index, document) in self.documents.iter().enumerate() {
            let score = document_score(&document.key, &tokens);
            let Some(score) = score else { continue };
            let bonus = self
                .history
                .get(&document.path)
                .map_or(0, |usage| usage.bonus(now));
            let candidate = (
                score.saturating_sub(bonus),
                std::cmp::Reverse(bonus),
                &document.key,
                index,
            );
            let position = best.partition_point(|previous| previous < &candidate);
            if position < MAX_RESULTS {
                best.insert(position, candidate);
                best.truncate(MAX_RESULTS);
            }
        }
        best.into_iter()
            .map(|(score, _, _, index)| {
                let document = &self.documents[index];
                App {
                    name: document
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    key: document.key.clone(),
                    target: Target::Document(document.path.clone()),
                    icon: None,
                    file_rank: Some(score),
                }
            })
            .collect()
    }

    fn opened(&mut self, path: PathBuf, now: u64) {
        if !self.history.contains_key(&path) && self.history.len() >= MAX_HISTORY {
            if let Some(oldest) = self
                .history
                .iter()
                .min_by_key(|(_, usage)| usage.last_opened)
                .map(|(path, _)| path.clone())
            {
                self.history.remove(&oldest);
            }
        }
        let usage = self.history.entry(path).or_default();
        usage.count = usage.count.saturating_add(1);
        usage.last_opened = now;
    }

    fn load(path: &Path, policy: &Policy) -> Self {
        let loaded = (|| {
            let file = fs::File::open(path).ok()?;
            if file.metadata().ok()?.len() > MAX_CACHE_BYTES {
                return None;
            }
            let cache: Cache = serde_json::from_reader(file.take(MAX_CACHE_BYTES)).ok()?;
            (cache.version == 1).then_some(cache)
        })();
        let Some(cache) = loaded else {
            return Self::default();
        };
        let mut index = Self {
            limited: cache.limited,
            ..Self::default()
        };
        for path in cache.paths.into_iter().take(MAX_FILES) {
            if !policy.allows(&path) || !supported(&path) {
                continue;
            }
            if !index.insert(path) {
                break;
            }
        }
        for (path, usage) in cache.history.into_iter().take(MAX_HISTORY) {
            if policy.allows(&path) && supported(&path) {
                index.history.insert(path, usage);
            }
        }
        index
    }

    fn save(&self, path: &Path) -> std::io::Result<()> {
        let cache = Cache {
            version: 1,
            paths: self
                .documents
                .iter()
                .map(|document| document.path.clone())
                .collect(),
            history: self
                .history
                .iter()
                .map(|(path, usage)| {
                    (
                        path.clone(),
                        Usage {
                            count: usage.count,
                            last_opened: usage.last_opened,
                        },
                    )
                })
                .collect(),
            limited: self.limited,
        };
        let bytes = serde_json::to_vec(&cache)?;
        if bytes.len() as u64 > MAX_CACHE_BYTES {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            let source: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let destination: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            unsafe {
                MoveFileExW(
                    PCWSTR(source.as_ptr()),
                    PCWSTR(destination.as_ptr()),
                    MOVEFILE_REPLACE_EXISTING,
                )
            }
            .map_err(std::io::Error::from)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

struct Policy {
    excluded: Vec<String>,
}

impl Policy {
    fn system() -> Self {
        let excluded = [
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
            "LOCALAPPDATA",
            "APPDATA",
        ]
        .iter()
        .filter_map(std::env::var_os)
        .map(|path| {
            path.to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_lowercase()
        })
        .collect();
        Self { excluded }
    }

    fn allows(&self, path: &Path) -> bool {
        if !path.is_absolute() {
            return false;
        }
        let key = path.to_string_lossy().replace('/', "\\").to_lowercase();
        if key.starts_with("\\\\")
            || self.excluded.iter().any(|root| {
                key == *root
                    || key
                        .strip_prefix(root)
                        .is_some_and(|tail| tail.starts_with('\\'))
            })
        {
            return false;
        }
        !key.split('\\').enumerate().any(|(depth, name)| {
            name.starts_with('.')
                || name.starts_with('$')
                || matches!(
                    name,
                    "appdata"
                        | "node_modules"
                        | "target"
                        | "build"
                        | "dist"
                        | "vendor"
                        | "venv"
                        | "__pycache__"
                )
                || (depth == 1
                    && matches!(
                        name,
                        "windows"
                            | "windows.old"
                            | "program files"
                            | "program files (x86)"
                            | "programdata"
                            | "recovery"
                            | "system volume information"
                    ))
        })
    }
}

pub(super) fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "pdf"
                    | "doc"
                    | "docx"
                    | "rtf"
                    | "txt"
                    | "md"
                    | "markdown"
                    | "rst"
                    | "org"
                    | "odt"
                    | "ods"
                    | "odp"
                    | "xls"
                    | "xlsx"
                    | "csv"
                    | "ppt"
                    | "pptx"
                    | "epub"
                    | "tex"
            )
        })
}

pub(super) fn available(path: &Path) -> bool {
    supported(path)
        && fs::symlink_metadata(path).is_ok_and(|metadata| {
            metadata.is_file() && metadata.file_attributes() & SKIP_ATTRIBUTES == 0
        })
}

struct Scanner {
    roots: VecDeque<PathBuf>,
    priority: Option<PathBuf>,
    skip_priority: bool,
    stack: Vec<ReadDir>,
    visited: usize,
    index: Index,
}

impl Scanner {
    fn new(roots: Vec<PathBuf>, priority: Option<PathBuf>) -> Self {
        Self {
            roots: roots.into(),
            priority,
            skip_priority: false,
            stack: Vec::new(),
            visited: 0,
            index: Index::default(),
        }
    }

    fn step(&mut self, policy: &Policy) -> bool {
        let deadline = Instant::now() + Duration::from_millis(8);
        while Instant::now() < deadline {
            if self.stack.is_empty() {
                let Some(root) = self.roots.pop_front() else {
                    return true;
                };
                self.skip_priority = self
                    .priority
                    .as_ref()
                    .is_some_and(|priority| *priority != root);
                if policy.allows(&root) {
                    if let Ok(directory) = fs::read_dir(root) {
                        self.stack.push(directory);
                    }
                }
                continue;
            }
            let Some(entry) = self.stack.last_mut().unwrap().next() else {
                self.stack.pop();
                continue;
            };
            self.visited += 1;
            if self.visited >= MAX_VISITED {
                self.index.limited = true;
                return true;
            }
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if (self.skip_priority
                && self
                    .priority
                    .as_ref()
                    .is_some_and(|priority| path.starts_with(priority)))
                || !policy.allows(&path)
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.file_attributes() & SKIP_ATTRIBUTES != 0 {
                continue;
            }
            if metadata.is_dir() {
                if self.stack.len() >= 64 {
                    self.index.limited = true;
                    continue;
                }
                if let Ok(directory) = fs::read_dir(path) {
                    self.stack.push(directory);
                }
            } else if metadata.is_file() && supported(&path) && !self.index.insert(path) {
                return true;
            }
        }
        false
    }
}

fn roots() -> (Vec<PathBuf>, Option<PathBuf>) {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let mut roots: Vec<_> = home.clone().into_iter().collect();
    let drives = unsafe { GetLogicalDrives() };
    for letter in b'A'..=b'Z' {
        if drives & (1 << (letter - b'A')) == 0 {
            continue;
        }
        let root = format!("{}:\\", letter as char);
        let wide: Vec<_> = root.encode_utf16().chain(Some(0)).collect();
        if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } == 3 {
            roots.push(PathBuf::from(root));
        }
    }
    (roots, home)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn run(receiver: Receiver<Command>, publish: impl Fn(Reply)) {
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
    let cache_path = crate::hosted::data_directory()
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".termielle"))
        })
        .map(|root| root.join("launcher-files.json"));
    run_with(receiver, publish, roots(), cache_path);
}

fn run_with(
    receiver: Receiver<Command>,
    publish: impl Fn(Reply),
    locations: (Vec<PathBuf>, Option<PathBuf>),
    cache_path: Option<PathBuf>,
) {
    let policy = Policy::system();
    let mut index = Index::default();
    let mut scanner: Option<Scanner> = None;
    let mut initialized = false;
    let mut last_scan = Instant::now();
    let mut last_publish = Instant::now();
    let mut request = (0, String::new());
    loop {
        let command = if scanner.is_some() {
            receiver.recv_timeout(Duration::from_millis(8))
        } else {
            receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        let mut changed = false;
        let mut opened = false;
        match command {
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
            Ok(first) => {
                for command in std::iter::once(first).chain(receiver.try_iter().take(63)) {
                    match command {
                        Command::Search(generation, query) => {
                            request = (generation, query);
                            changed = true;
                        }
                        Command::Opened(path) => {
                            index.opened(path, now());
                            opened = true;
                        }
                    }
                }
            }
        }
        if !initialized {
            if let Some(path) = &cache_path {
                index = Index::load(path, &policy);
            }
            initialized = true;
            scanner = Some(Scanner::new(locations.0.clone(), locations.1.clone()));
        } else if changed && scanner.is_none() && last_scan.elapsed() >= Duration::from_secs(300) {
            scanner = Some(Scanner::new(locations.0.clone(), locations.1.clone()));
        }
        if scanner.as_mut().is_some_and(|scan| scan.step(&policy)) {
            let mut completed = scanner.take().unwrap().index;
            completed.history = std::mem::take(&mut index.history);
            index = completed;
            last_scan = Instant::now();
            if let Some(path) = &cache_path {
                let _ = index.save(path);
            }
            changed = true;
        } else if opened && scanner.is_none() {
            if let Some(path) = &cache_path {
                let _ = index.save(path);
            }
        }
        if changed || (scanner.is_some() && last_publish.elapsed() >= Duration::from_millis(500)) {
            let source = if index.documents.is_empty() {
                scanner.as_ref().map_or(&index, |scan| &scan.index)
            } else {
                &index
            };
            let status = format!(
                "{} documents{}{}",
                source.documents.len(),
                if scanner.is_some() {
                    " · indexing…"
                } else {
                    ""
                },
                if source.limited {
                    " · index limit reached"
                } else {
                    ""
                }
            );
            publish(Reply {
                generation: request.0,
                entries: source.search(&request.1, now()),
                status,
            });
            last_publish = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_names_parent_tokens_and_extension_search_are_ranked_and_bounded() {
        let mut index = Index::default();
        index.insert(PathBuf::from("C:/Work/Acme/Design Proposal.pdf"));
        index.insert(PathBuf::from("C:/Notes/Other.docx"));
        assert_eq!(
            index.search("acme dsgn pdf", 0)[0].name,
            "Design Proposal.pdf"
        );
        assert_eq!(index.search("DOCX", 0)[0].name, "Other.docx");
        assert!(index.search("unrelated", 0).is_empty());
        assert!(index.search("  ", 0).is_empty());
        for number in 0..100 {
            index.insert(PathBuf::from(format!("C:/Work/Report {number}.pdf")));
        }
        assert_eq!(index.search("report", 0).len(), MAX_RESULTS);
    }

    #[test]
    fn successful_open_frequency_and_recency_promote_matching_documents() {
        let mut index = Index::default();
        index.insert(PathBuf::from("C:/Alpha/Report.pdf"));
        index.insert(PathBuf::from("C:/Beta/Report.pdf"));
        let path = PathBuf::from("C:/Beta/Report.pdf");
        index.opened(path.clone(), 1_000);
        assert_eq!(
            index.search("report", 1_001)[0].target,
            Target::Document(path)
        );
        assert_eq!(
            Usage {
                count: 2,
                last_opened: 0
            }
            .bonus(1),
            8
        );
        assert_eq!(
            Usage {
                count: 2,
                last_opened: 0
            }
            .bonus(700_000),
            0
        );
    }

    #[test]
    fn scanner_skips_system_hidden_and_build_paths() {
        let directory = tempfile::Builder::new()
            .prefix("files-test-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let root = directory.path();
        for folder in [
            "Documents",
            "Windows",
            "AppData",
            "node_modules",
            "target",
            ".git",
        ] {
            fs::create_dir(root.join(folder)).unwrap();
            fs::write(root.join(folder).join("report.pdf"), b"not parsed").unwrap();
        }
        fs::write(root.join("Documents/program.exe"), b"not indexed").unwrap();
        fs::write(root.join("Documents/hidden.pdf"), b"not indexed").unwrap();
        let hidden: Vec<_> = root
            .join("Documents/hidden.pdf")
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            windows::Win32::Storage::FileSystem::SetFileAttributesW(
                PCWSTR(hidden.as_ptr()),
                windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
            )
        }
        .unwrap();
        let policy = Policy {
            excluded: vec![
                root.join("Windows")
                    .to_string_lossy()
                    .replace('/', "\\")
                    .to_lowercase(),
            ],
        };
        let mut scanner = Scanner::new(vec![root.to_path_buf()], None);
        while !scanner.step(&policy) {}
        assert_eq!(scanner.index.documents.len(), 1);
        assert!(
            scanner.index.documents[0]
                .path
                .ends_with("Documents/report.pdf")
        );
        assert!(!policy.allows(Path::new("C:/Program Files/App/readme.txt")));
        assert!(!policy.allows(Path::new("../report.pdf")));
        assert!(!policy.allows(Path::new("//server/share/report.pdf")));
        assert!(policy.allows(Path::new("C:/Docs/Windows/Book.pdf")));
    }

    #[test]
    fn persistent_index_roundtrips_usage_and_rejects_invalid_cache() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("index.json");
        let policy = Policy {
            excluded: Vec::new(),
        };
        let mut index = Index::default();
        let path = PathBuf::from("C:/Work/Report.pdf");
        index.insert(path.clone());
        index.opened(path.clone(), 123);
        index.save(&cache).unwrap();
        index.save(&cache).unwrap();
        let loaded = Index::load(&cache, &policy);
        assert_eq!(loaded.documents.len(), 1);
        assert_eq!(loaded.history[&path].last_opened, 123);
        fs::write(&cache, b"not json").unwrap();
        assert!(Index::load(&cache, &policy).documents.is_empty());
    }

    #[test]
    fn prepared_index_and_history_have_hard_limits() {
        let mut index = Index::default();
        for number in 0..MAX_FILES + 1 {
            if !index.insert(PathBuf::from(format!("C:/Docs/{number}.pdf"))) {
                break;
            }
        }
        assert!(index.limited);
        assert!(index.documents.len() <= MAX_FILES);
        assert!(index.bytes <= MAX_INDEX_BYTES);
        for number in 0..MAX_HISTORY + 1 {
            index.opened(PathBuf::from(format!("C:/{number}.pdf")), number as u64);
        }
        assert_eq!(index.history.len(), MAX_HISTORY);
        assert!(!index.history.contains_key(Path::new("C:/0.pdf")));
        let started = Instant::now();
        assert!(!index.search("docs 42", 0).is_empty());
        println!(
            "Searched {} document paths in {:?} ({} prepared bytes)",
            index.documents.len(),
            started.elapsed(),
            index.bytes
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn cache_cannot_reintroduce_system_or_executable_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.json");
        let cache = Cache {
            version: 1,
            paths: vec![
                PathBuf::from("C:/Windows/secret.pdf"),
                PathBuf::from("C:/Work/launch.exe"),
                PathBuf::from("C:/Work/Good.pdf"),
            ],
            history: HashMap::new(),
            limited: false,
        };
        fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        let index = Index::load(
            &path,
            &Policy {
                excluded: Vec::new(),
            },
        );
        assert_eq!(index.documents.len(), 1);
        assert!(index.documents[0].path.ends_with("Work/Good.pdf"));
    }

    #[test]
    fn unicode_and_path_queries_do_not_require_document_contents() {
        let mut index = Index::default();
        index.insert(PathBuf::from("C:/Clients/日本語/Équipe Résumé.docx"));
        assert_eq!(index.search("ÉQUI 日本語 docx", 0).len(), 1);
        assert_eq!(index.search("clients/日本語", 0).len(), 1);
    }

    #[test]
    fn worker_indexes_fixture_files_answers_queries_and_stops_on_disconnect() {
        let directory = tempfile::Builder::new()
            .prefix("files-worker-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let document = directory.path().join("Design Proposal.pdf");
        fs::write(&document, b"not a PDF; contents must never be parsed").unwrap();
        let cache = directory.path().join("cache.json");
        let locations = (vec![directory.path().to_path_buf()], None);
        let (sender, receiver) = std::sync::mpsc::channel();
        let (replies, results) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            run_with(
                receiver,
                |reply| {
                    let _ = replies.send(reply);
                },
                locations,
                Some(cache),
            )
        });
        sender.send(Command::Search(1, "dsgn pdf".into())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let reply = results
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if !reply.status.contains("indexing") {
                assert_eq!(reply.entries[0].target, Target::Document(document.clone()));
                break;
            }
        }
        sender.send(Command::Opened(document.clone())).unwrap();
        sender.send(Command::Search(2, "proposal".into())).unwrap();
        let reply = results.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(reply.generation, 2);
        assert_eq!(reply.entries[0].target, Target::Document(document));
        drop(sender);
        thread.join().unwrap();
        assert!(directory.path().join("cache.json").exists());
    }
}
