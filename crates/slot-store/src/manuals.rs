use std::io;
use std::path::{Path, PathBuf};

use crate::Cart;

pub const MANUALS_DIR: &str = "Manuals";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualPage {
    pub number: u64,
    pub path: PathBuf,
}

/// Finds a game's Icecream-exported PNG pages in numeric order.
///
/// The game directory and page filenames are matched case-insensitively.
/// Missing directories return an empty list; conflicting page numbers and
/// filesystem errors are reported rather than choosing an arbitrary page.
pub fn manual_pages(root: &Path, cart: &Cart) -> io::Result<Vec<ManualPage>> {
    let platform_dir = root.join(MANUALS_DIR).join(cart.platform.dir_name());
    let Some(dir) = game_dir(&platform_dir, &cart.stem)? else {
        return Ok(Vec::new());
    };

    let mut pages = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(number) = page_number(name, &cart.stem)? else {
            continue;
        };
        if std::fs::metadata(entry.path())?.is_file() {
            pages.push(ManualPage {
                number,
                path: entry.path(),
            });
        }
    }
    pages.sort_by_key(|page| page.number);
    if let Some(pair) = pages
        .windows(2)
        .find(|pair| pair[0].number == pair[1].number)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "duplicate manual page {}: {} and {}",
                pair[0].number,
                pair[0].path.display(),
                pair[1].path.display()
            ),
        ));
    }
    Ok(pages)
}

fn game_dir(platform_dir: &Path, stem: &str) -> io::Result<Option<PathBuf>> {
    let entries = match std::fs::read_dir(platform_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut found = None;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if name
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(stem))
            && std::fs::metadata(entry.path())?.is_dir()
        {
            if found.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "multiple manual directories match {stem} in {}",
                        platform_dir.display()
                    ),
                ));
            }
            found = Some(entry.path());
        }
    }
    Ok(found)
}

fn page_number(name: &str, stem: &str) -> io::Result<Option<u64>> {
    let Some(prefix) = name.get(..stem.len()) else {
        return Ok(None);
    };
    if !prefix.eq_ignore_ascii_case(stem) {
        return Ok(None);
    }
    let rest = &name[stem.len()..];
    let Some(marker) = rest.get(..6) else {
        return Ok(None);
    };
    if !marker.eq_ignore_ascii_case("_page_") {
        return Ok(None);
    }
    let Some((number, suffix)) = rest[6..].split_once('_') else {
        return Ok(None);
    };
    if !suffix.eq_ignore_ascii_case("image_1.png")
        || number.is_empty()
        || !number.bytes().all(|b| b.is_ascii_digit())
    {
        return Ok(None);
    }
    number.parse().map(Some).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid manual page number in {name}: {e}"),
        )
    })
}
