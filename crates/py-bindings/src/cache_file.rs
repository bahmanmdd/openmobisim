//! What a network's and a parkings' cache files share (S294, I-as): the stamp of the build that
//! writes them, writing a file whole or not at all, and reading one back only when its first
//! bytes say it holds what is asked for and its header says this build wrote it from the source
//! the reader expects. Each file's layout is `openmobisim_core_graph::binary`'s.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_graph::binary::{self, Content, Header, Reader};
use openmobisim_core_types::GIT_SHA;

/// What tells this build from another: its commit, platform and profile, and the compiled
/// module's size and modification time, which a rebuild changes even without a commit (changed
/// code may import another network, or read other parkings). Computed once.
pub(crate) fn build_stamp(py: Python<'_>) -> String {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP
        .get_or_init(|| {
            let file = py
                .import("openmobisim._core")
                .and_then(|m| m.getattr("__file__"))
                .and_then(|f| f.extract::<PathBuf>())
                .ok();
            let module = file.and_then(|f| std::fs::metadata(f).ok()).map_or_else(
                || "unknown".to_string(),
                |m| {
                    let modified = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos());
                    format!("{}:{modified}", m.len())
                },
            );
            let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
            format!("{GIT_SHA} {} {profile} {module}", env!("OPENMOBISIM_TARGET"))
        })
        .clone()
}

fn format_error(path: &Path, e: &binary::FormatError) -> PyErr {
    PyValueError::new_err(format!("{}: {e}", path.display()))
}

/// Write `bytes` to `path`, beside its final name first and then renamed, so that a reader
/// never sees half of it.
///
/// # Errors
///
/// `OSError` if the file cannot be written.
pub(crate) fn write_whole(py: Python<'_>, path: &Path, bytes: &[u8]) -> PyResult<()> {
    // A name of its own for each write, so that two writers of one file never share it.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let partial = path.with_file_name(format!(
        "{}.partial-{}-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("cache"),
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = py
        .detach(|| std::fs::write(&partial, bytes).and_then(|()| std::fs::rename(&partial, path)));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        PyOSError::new_err(format!("{}: {e}", path.display()))
    })
}

/// How a message names what a file holds, and what to do instead of reading it.
const fn named(content: Content) -> (&'static str, &'static str) {
    match content {
        Content::Network => ("a network", "import the network again"),
        Content::Parkings => ("parkings", "read the parkings again"),
    }
}

/// Read the file at `path` holding `content`: its header, and then, if this build wrote it
/// (and from `source`, when given), its body by `body`, to the last byte.
///
/// # Errors
///
/// `ValueError` for a file that does not hold `content`, is damaged, was written by another
/// version or build of openmobisim, or from another `source`; `OSError` if it cannot be read.
pub(crate) fn read_whole<T: Send>(
    py: Python<'_>,
    path: &Path,
    content: Content,
    source: Option<&str>,
    body: impl FnOnce(&mut Reader<'_>) -> binary::Result<T> + Send,
) -> PyResult<T> {
    let here = Header::current(&build_stamp(py), "", f64::NAN);
    let bytes = py
        .detach(|| std::fs::read(path))
        .map_err(|e| PyOSError::new_err(format!("{}: {e}", path.display())))?;
    let (header, value) = py
        .detach(|| {
            let mut r = Reader::of(&bytes, content)?;
            let header = Header::read(&mut r)?;
            if header.differs_from(&here).is_some() || source.is_some_and(|s| s != header.source) {
                // Refused before the content is read; the message is made below.
                return Ok((header, None));
            }
            let value = body(&mut r)?;
            r.finish()?;
            Ok::<_, binary::FormatError>((header, Some(value)))
        })
        .map_err(|e| format_error(path, &e))?;
    let (what, again) = named(content);
    if let Some(differs) = header.differs_from(&here) {
        return Err(PyValueError::new_err(format!(
            "{}: written by another build of openmobisim ({differs}); {again}",
            path.display()
        )));
    }
    let Some(value) = value else {
        return Err(PyValueError::new_err(format!(
            "{}: {what} made from another source ({:?}, not {:?})",
            path.display(),
            header.source,
            source.unwrap_or_default()
        )));
    };
    Ok(value)
}

/// The header of a file holding `content`, read from its first bytes without reading the rest:
/// `format_version`, `code_version`, `defaults_version`, `version`, `build`, `source` and
/// `made_s` (the seconds making what it holds took), and `current`, whether this build would
/// read it. Not checked (the whole file is, when read).
///
/// # Errors
///
/// `ValueError` for a file that does not hold `content` in this format; `OSError` if it cannot
/// be read.
pub(crate) fn read_header<'py>(
    py: Python<'py>,
    path: &Path,
    content: Content,
) -> PyResult<Bound<'py, PyDict>> {
    use std::io::Read;
    let mut start = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(1 << 16).read_to_end(&mut start))
        .map_err(|e| PyOSError::new_err(format!("{}: {e}", path.display())))?;
    let header = Header::peek_of(&start, content).map_err(|e| format_error(path, &e))?;
    let here = Header::current(&build_stamp(py), "", f64::NAN);
    let d = PyDict::new(py);
    d.set_item("format_version", header.format_version)?;
    d.set_item("code_version", header.code_version)?;
    d.set_item("defaults_version", header.defaults_version)?;
    d.set_item("version", &header.crate_version)?;
    d.set_item("build", &header.build)?;
    d.set_item("source", &header.source)?;
    d.set_item("made_s", header.made_seconds)?;
    d.set_item("current", header.differs_from(&here).is_none())?;
    Ok(d)
}
