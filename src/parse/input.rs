//! Reads files (memory mapped when large) and stdin into memory.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use memmap2::Mmap;
use miette::Result;

use crate::cli::Config;
use crate::error::SilvError;

/// Files at or above this size are memory mapped instead of copied.
const MMAP_THRESHOLD: u64 = 8 * 1024 * 1024;

pub const STDIN_LABEL: &str = "<stdin>";

enum SourceData {
    Owned(Vec<u8>),
    Mapped(Mmap),
}

/// One input source with its bytes.
pub struct RawSource {
    pub label: String,
    data: SourceData,
}

impl RawSource {
    pub fn bytes(&self) -> &[u8] {
        match &self.data {
            SourceData::Owned(bytes) => bytes,
            SourceData::Mapped(map) => map,
        }
    }

    pub fn len(&self) -> usize {
        self.bytes().len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes().is_empty()
    }
}

/// All sources loaded for a run.
pub struct InputBundle {
    pub sources: Vec<RawSource>,
    pub read_stdin: bool,
}

#[cfg(test)]
impl RawSource {
    /// Test double: an in-memory source with the given label and bytes.
    pub fn for_test(label: &str, bytes: Vec<u8>) -> Self {
        Self {
            label: label.to_owned(),
            data: SourceData::Owned(bytes),
        }
    }
}

impl InputBundle {
    pub fn total_bytes(&self) -> usize {
        self.sources.iter().map(RawSource::len).sum()
    }
}

pub fn read_inputs(config: &Config) -> Result<InputBundle> {
    let mut bundle = InputBundle {
        sources: Vec::new(),
        read_stdin: false,
    };

    let mut files: Vec<PathBuf> = Vec::new();
    let mut wants_stdin = config.files.is_empty();

    for arg in &config.files {
        let text = arg.to_string_lossy().replace('\\', "/");
        if text == "-" {
            wants_stdin = true;
            continue;
        }
        let path = Path::new(arg);
        if path.is_dir() {
            collect_directory(path, config, &mut files)?;
        } else {
            files.push(path.to_path_buf());
        }
    }

    let matchers = build_matchers(config);
    files.sort();
    files.dedup();
    let files: Vec<PathBuf> = files
        .into_iter()
        .filter(|path| !matches_any(path, &matchers.exclude))
        .filter(|path| matchers.include.is_empty() || matches_any(path, &matchers.include))
        .collect();

    for path in &files {
        bundle.sources.push(read_file(path)?);
    }

    if wants_stdin {
        bundle.sources.push(read_stdin()?);
        bundle.read_stdin = true;
    }

    bundle.sources.retain(|source| !source.is_empty());
    if bundle.sources.is_empty() {
        return Err(SilvError::InputEmpty.into());
    }
    Ok(bundle)
}

struct Matchers {
    include: Vec<globset::GlobMatcher>,
    exclude: Vec<globset::GlobMatcher>,
}

fn build_matchers(config: &Config) -> Matchers {
    Matchers {
        include: compile_all(&config.include),
        exclude: compile_all(&config.exclude),
    }
}

fn compile_all(patterns: &[String]) -> Vec<globset::GlobMatcher> {
    patterns
        .iter()
        .filter_map(|pattern| globset::Glob::new(pattern).ok())
        .map(|glob| glob.compile_matcher())
        .collect()
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn matches_any(path: &Path, matchers: &[globset::GlobMatcher]) -> bool {
    let full = normalized(path);
    let name = file_name(path);
    matchers.iter().any(|m| m.is_match(&full) || m.is_match(&name))
}

fn collect_directory(dir: &Path, config: &Config, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|cause| SilvError::InputRead {
        path: dir.display().to_string(),
        cause,
    })?;
    for entry in entries {
        let entry = entry.map_err(|cause| SilvError::InputRead {
            path: dir.display().to_string(),
            cause,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|cause| SilvError::InputRead {
            path: path.display().to_string(),
            cause,
        })?;
        if file_type.is_dir() {
            if config.recursive {
                collect_directory(&path, config, out)?;
            }
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<RawSource> {
    let label = path.display().to_string();
    let file = File::open(path).map_err(|cause| SilvError::InputRead {
        path: label.clone(),
        cause,
    })?;
    let size = file
        .metadata()
        .map_err(|cause| SilvError::InputRead {
            path: label.clone(),
            cause,
        })?
        .len();
    let data = if size >= MMAP_THRESHOLD {
        let map = unsafe { memmap2::MmapOptions::new().map(&file) }.map_err(|cause| SilvError::InputRead {
            path: label.clone(),
            cause,
        })?;
        SourceData::Mapped(map)
    } else {
        let mut bytes = Vec::with_capacity(size as usize);
        file.take(size + 1)
            .read_to_end(&mut bytes)
            .map_err(|cause| SilvError::InputRead {
                path: label.clone(),
                cause,
            })?;
        SourceData::Owned(bytes)
    };
    Ok(RawSource { label, data })
}

fn read_stdin() -> Result<RawSource> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .read_to_end(&mut bytes)
        .map_err(|cause| SilvError::InputRead {
            path: STDIN_LABEL.to_owned(),
            cause,
        })?;
    Ok(RawSource {
        label: STDIN_LABEL.to_owned(),
        data: SourceData::Owned(bytes),
    })
}
