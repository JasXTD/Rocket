//! Linux update archives: gzip-compressed POSIX ustar, with every entry inside one top
//! folder, `Rocket-<version>-linux-x64`. The system's gzip decompresses the archive; this
//! reader checks each entry and writes the files itself, so a download can't create a
//! link, a device, a file outside its folder or one that replaces another. Windows reads
//! its ZIP with the system's ZIP reader instead.
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Write},
    path::PathBuf,
};

/// The same limits as the unpacked-release checks in the desktop app.
pub(super) const MAX_FILE: u64 = 256 * 1024 * 1024;
pub(super) const MAX_TREE: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 20000;
const BLOCK: usize = 512;

fn refuse(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// Reads decompressed archive bytes, fed in chunks of any size, into `output`: a new
/// folder that ends up holding the top folder's contents. Call `finish` at the end.
pub(super) struct Unpacker {
    output: PathBuf,
    top: Option<String>,
    header: Vec<u8>,
    file: Option<(File, u64)>,
    padding: u64,
    entries: usize,
    total: u64,
    names: BTreeSet<String>,
    zero_blocks: usize,
}

impl Unpacker {
    pub(super) fn new(output: PathBuf) -> io::Result<Self> {
        fs::create_dir(&output)?;
        Ok(Self {
            output,
            top: None,
            header: Vec::with_capacity(BLOCK),
            file: None,
            padding: 0,
            entries: 0,
            total: 0,
            names: BTreeSet::new(),
            zero_blocks: 0,
        })
    }

    pub(super) fn feed(&mut self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            if let Some((file, left)) = &mut self.file {
                let count = data.len().min(usize::try_from(*left).unwrap_or(usize::MAX));
                file.write_all(&data[..count])?;
                *left -= count as u64;
                data = &data[count..];
                if *left == 0 {
                    self.file = None;
                }
                continue;
            }
            if self.padding > 0 {
                let count = data.len().min(self.padding as usize);
                if data[..count].iter().any(|&b| b != 0) {
                    return Err(refuse("archive padding is not empty"));
                }
                self.padding -= count as u64;
                data = &data[count..];
                continue;
            }
            let count = data.len().min(BLOCK - self.header.len());
            self.header.extend_from_slice(&data[..count]);
            data = &data[count..];
            if self.header.len() == BLOCK {
                let block = std::mem::replace(&mut self.header, Vec::with_capacity(BLOCK));
                self.entry(&block)?;
            }
        }
        Ok(())
    }

    /// Checks that the archive ended where its end marker says.
    pub(super) fn finish(self) -> io::Result<()> {
        if self.zero_blocks < 2 || self.file.is_some() || self.padding > 0 {
            return Err(refuse("the archive is incomplete"));
        }
        if self.header.iter().any(|&b| b != 0) || self.top.is_none() {
            return Err(refuse("the archive is incomplete"));
        }
        Ok(())
    }

    fn entry(&mut self, block: &[u8]) -> io::Result<()> {
        if block.iter().all(|&b| b == 0) {
            self.zero_blocks += 1;
            return Ok(());
        }
        // Only zero blocks may follow the end marker, and a lone zero block is not one.
        if self.zero_blocks > 0 {
            return Err(refuse("data follows the archive's end"));
        }
        if &block[257..263] != b"ustar\0" || &block[263..265] != b"00" {
            return Err(refuse("the archive is not POSIX ustar"));
        }
        let stored = octal(&block[148..156]).ok_or_else(|| refuse("an entry has no checksum"))?;
        let sum: u64 = block
            .iter()
            .enumerate()
            .map(|(at, &b)| u64::from(if (148..156).contains(&at) { b' ' } else { b }))
            .sum();
        if stored != sum {
            return Err(refuse("an entry's checksum is wrong"));
        }
        self.entries += 1;
        if self.entries > MAX_ENTRIES {
            return Err(refuse("the archive has too many entries"));
        }
        let name = text(&block[0..100])?;
        let prefix = text(&block[345..500])?;
        let full = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let size = octal(&block[124..136]).ok_or_else(|| refuse("an entry has no size"))?;
        match block[156] {
            b'0' | 0 => self.file_entry(&full, size),
            b'5' if size == 0 => self.directory_entry(full.strip_suffix('/').unwrap_or(&full)),
            _ => Err(refuse(
                "the archive contains a link, a device or another special entry",
            )),
        }
    }

    /// The entry's path inside the top folder, or None for the top folder itself. Every
    /// entry must sit in the same top folder, named `Rocket-<x.y.z>-linux-x64`.
    fn inside<'a>(&mut self, path: &'a str) -> io::Result<Option<&'a str>> {
        let (top, rest) = match path.split_once('/') {
            Some((top, rest)) => (top, Some(rest)),
            None => (path, None),
        };
        match &self.top {
            Some(known) if known != top => {
                return Err(refuse("the archive has more than one top folder"));
            }
            Some(_) => {}
            None => {
                let version = top
                    .strip_prefix("Rocket-")
                    .and_then(|t| t.strip_suffix(if cfg!(target_os = "macos") { "-macos-arm64" } else { "-linux-x64" }))
                    .ok_or_else(|| refuse("the archive's top folder does not match this platform"))?;
                if super::UpdateVersion::parse(version).is_none() || version.starts_with('v') {
                    return Err(refuse("the archive's top folder has an invalid release version"));
                }
                self.top = Some(top.to_owned());
            }
        }
        let Some(rest) = rest else { return Ok(None) };
        if rest.is_empty()
            || rest.len() >= 1024
            || rest.split('/').any(|part| {
                part.is_empty()
                    || part == "."
                    || part == ".."
                    || part.bytes().any(|b| b < 32 || b == 127 || b == b'\\')
            })
        {
            return Err(refuse("an entry has an unsafe path"));
        }
        if !self.names.insert(rest.to_lowercase()) {
            return Err(refuse("an entry appears twice"));
        }
        Ok(Some(rest))
    }

    fn directory_entry(&mut self, path: &str) -> io::Result<()> {
        if let Some(rest) = self.inside(path)? {
            self.directories(rest)?;
        }
        Ok(())
    }

    fn file_entry(&mut self, path: &str, size: u64) -> io::Result<()> {
        let rest = self
            .inside(path)?
            .ok_or_else(|| refuse("the archive's top folder is a file"))?;
        if size > MAX_FILE {
            return Err(refuse("a file is larger than the update allows"));
        }
        self.total = self
            .total
            .checked_add(size)
            .filter(|&total| total <= MAX_TREE)
            .ok_or_else(|| refuse("the unpacked update is larger than allowed"))?;
        if let Some((parent, _)) = rest.rsplit_once('/') {
            self.directories(parent)?;
        }
        // A new file every time: nothing in the folder can be replaced or written through.
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(self.output.join(rest))?;
        self.padding = (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64;
        if size > 0 {
            self.file = Some((file, size));
        }
        Ok(())
    }

    /// Creates the folders of `path` under the output, refusing anything there that is
    /// not a folder. The reader never creates a link, so none can be in the way.
    fn directories(&mut self, path: &str) -> io::Result<()> {
        let mut at = self.output.clone();
        for part in path.split('/') {
            at.push(part);
            match fs::symlink_metadata(&at) {
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => return Err(refuse("an entry is in the way of another")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&at)?,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// A NUL-terminated header field as UTF-8.
fn text(field: &[u8]) -> io::Result<&str> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..end]).map_err(|_| refuse("an entry name is not UTF-8"))
}

/// An octal header number, padded with spaces or NULs as tar writes them.
fn octal(field: &[u8]) -> Option<u64> {
    let digits = field.split(|&b| b == 0).next()?.trim_ascii();
    if digits.is_empty() {
        return None;
    }
    digits.iter().try_fold(0u64, |value, &digit| {
        if !(b'0'..=b'7').contains(&digit) {
            return None;
        }
        value.checked_mul(8)?.checked_add(u64::from(digit - b'0'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One ustar header, as GNU tar --format=ustar writes it.
    fn header(path: &str, kind: u8, size: usize) -> Vec<u8> {
        let mut block = vec![0u8; BLOCK];
        let (prefix, name) = if path.len() > 100 {
            let cut = path[..=155.min(path.len() - 1)]
                .rfind('/')
                .expect("a long ustar path splits at a folder");
            (&path[..cut], &path[cut + 1..])
        } else {
            ("", path)
        };
        assert!(name.len() <= 100 && prefix.len() <= 155, "{path}");
        block[..name.len()].copy_from_slice(name.as_bytes());
        block[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        block[100..108].copy_from_slice(b"0000644\0");
        block[108..116].copy_from_slice(b"0000000\0");
        block[116..124].copy_from_slice(b"0000000\0");
        block[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
        block[136..148].copy_from_slice(b"00000000000\0");
        block[156] = kind;
        block[257..263].copy_from_slice(b"ustar\0");
        block[263..265].copy_from_slice(b"00");
        block[148..156].copy_from_slice(b"        ");
        let sum: u32 = block.iter().map(|&b| u32::from(b)).sum();
        block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        block
    }
    fn archive(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (path, kind, data) in entries {
            bytes.extend(header(path, *kind, data.len()));
            bytes.extend(*data);
            bytes.resize(bytes.len().next_multiple_of(BLOCK), 0);
        }
        bytes.extend([0u8; BLOCK * 2]);
        // tar pads to whole records of 20 blocks.
        bytes.resize(bytes.len().next_multiple_of(BLOCK * 20), 0);
        bytes
    }
    fn output() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rocket-unpack-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root.join("bundle")
    }
    /// Feeds the archive in awkward chunk sizes, as a pipe delivers it.
    fn unpack(bytes: &[u8], output: &std::path::Path) -> io::Result<()> {
        let mut unpacker = Unpacker::new(output.to_path_buf())?;
        for chunk in bytes.chunks(777) {
            unpacker.feed(chunk)?;
        }
        unpacker.finish()
    }
    #[cfg(not(target_os = "macos"))]
    const TOP: &str = "Rocket-9.0.0-linux-x64";
    #[cfg(target_os = "macos")]
    const TOP: &str = "Rocket-9.0.0-macos-arm64";

    #[test]
    fn a_release_folder_unpacks_with_its_files_and_folders() {
        let long = format!("{TOP}/docs/wiki/{}/page.md", "a".repeat(90));
        let bytes = archive(&[
            (TOP, b'5', b""),
            (&format!("{TOP}/rocket"), b'0', b"binary"),
            (&format!("{TOP}/docs/"), b'5', b""),
            (
                &format!("{TOP}/docs/資料.md"),
                b'0',
                "Unicode 🚀".as_bytes(),
            ),
            (&format!("{TOP}/empty.txt"), b'0', b""),
            (&long, b'0', &[7u8; 1500]),
        ]);
        let out = output();
        unpack(&bytes, &out).unwrap();
        assert_eq!(fs::read(out.join("rocket")).unwrap(), b"binary");
        assert_eq!(
            fs::read_to_string(out.join("docs/資料.md")).unwrap(),
            "Unicode 🚀"
        );
        assert_eq!(fs::read(out.join("empty.txt")).unwrap(), b"");
        let inner = long.strip_prefix(&format!("{TOP}/")).unwrap();
        assert_eq!(fs::read(out.join(inner)).unwrap(), vec![7u8; 1500]);
        fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn links_devices_escapes_and_doubles_are_refused() {
        let file = |path: String| (path, b'0', b"x".as_slice());
        let cases: Vec<Vec<(String, u8, &[u8])>> = vec![
            vec![(format!("{TOP}/link"), b'2', b"")],
            vec![(format!("{TOP}/hard"), b'1', b"")],
            vec![(format!("{TOP}/device"), b'3', b"")],
            vec![(format!("{TOP}/fifo"), b'6', b"")],
            vec![(format!("{TOP}/pax"), b'x', b"path=../../x\n")],
            vec![file(format!("{TOP}/../escape"))],
            vec![file(format!("{TOP}/a/./b"))],
            vec![file(format!("{TOP}//double-slash"))],
            vec![file("/etc/passwd".into())],
            vec![file("Rocket-9.0.0-windows-x64/rocket".into())],
            vec![file("Rocket-v9.0.0-linux-x64/rocket".into())],
            vec![
                file(format!("{TOP}/a")),
                file("Rocket-9.0.1-linux-x64/b".into()),
            ],
            vec![file(format!("{TOP}/Same")), file(format!("{TOP}/same"))],
            vec![file(format!("{TOP}/a")), file(format!("{TOP}/a/b"))],
            vec![file(format!("{TOP}/tab\tname"))],
            vec![(TOP.into(), b'0', b"top is a file")],
        ];
        for entries in cases {
            let refs: Vec<_> = entries
                .iter()
                .map(|(p, k, d)| (p.as_str(), *k, *d))
                .collect();
            let out = output();
            assert!(unpack(&archive(&refs), &out).is_err(), "{entries:?}");
            // Nothing escaped: only the output folder exists beside the fixture.
            let siblings: Vec<_> = fs::read_dir(out.parent().unwrap()).unwrap().collect();
            assert_eq!(siblings.len(), 1);
            fs::remove_dir_all(out.parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn damaged_and_truncated_archives_are_refused() {
        let good = archive(&[(&format!("{TOP}/rocket"), b'0', b"binary")]);
        let mut bad_sum = good.clone();
        bad_sum[0] ^= 1;
        let mut gnu_magic = good.clone();
        gnu_magic[257..265].copy_from_slice(b"ustar  \0");
        let mut trailing = good.clone();
        trailing.extend(header(&format!("{TOP}/late"), b'0', 0));
        let mut padding = good.clone();
        padding[BLOCK + 10] = 1;
        let oversized = header(&format!("{TOP}/huge"), b'0', (MAX_FILE + 1) as usize);
        for (name, bytes) in [
            ("checksum", bad_sum),
            ("old GNU magic", gnu_magic),
            ("data after the end", trailing),
            ("non-zero padding", padding),
            ("truncated", good[..BLOCK + 3].to_vec()),
            ("no end marker", good[..BLOCK * 2].to_vec()),
            ("oversized file", oversized),
            ("empty", Vec::new()),
        ] {
            let out = output();
            assert!(unpack(&bytes, &out).is_err(), "{name}");
            fs::remove_dir_all(out.parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn an_existing_output_folder_is_never_reused() {
        let out = output();
        fs::create_dir(&out).unwrap();
        assert!(Unpacker::new(out.clone()).is_err());
        fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }

    #[test]
    fn octal_fields_parse_as_tar_writes_them() {
        assert_eq!(octal(b"00000000017\0"), Some(15));
        assert_eq!(octal(b"     17 \0"), Some(15));
        assert_eq!(octal(b"0000008\0"), None);
        assert_eq!(octal(b"\0\0\0"), None);
        assert_eq!(octal(b"77777777777777777777777"), None);
    }
}
