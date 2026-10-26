use std::path::Path;

use super::*;

/// Raw bytes, read at their own offsets: a whole mapped image, or the
/// headers of a file.
impl Rva for [u8] {
    fn read(&self, rva: usize, buf: &mut [u8]) -> Option<()> {
        buf.copy_from_slice(self.get(rva..rva.checked_add(buf.len())?)?);
        Some(())
    }
}

const SECTION_RVA: usize = 0x2000;
const SECTION_FILE_OFFSET: usize = 0x200;
const SIZE_OF_IMAGE: usize = 0x3000;
const SECTION_ALIGNMENT: usize = 0x1000;
const FILE_ALIGNMENT: usize = 0x200;

/// What a test or fixture builder returns, so a fixture that does not
/// fit fails the test through `?` rather than a panic.
type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Copy `bytes` into `buf` at `at`.
fn put(buf: &mut [u8], at: usize, bytes: &[u8]) -> Fallible {
    let end = at.checked_add(bytes.len()).ok_or("offset overflows")?;
    buf.get_mut(at..end)
        .ok_or_else(|| format!("{at:#x}..{end:#x} is outside the fixture"))?
        .copy_from_slice(bytes);
    Ok(())
}

fn put16(buf: &mut [u8], at: usize, v: u16) -> Fallible {
    put(buf, at, &v.to_le_bytes())
}

fn put32(buf: &mut [u8], at: usize, v: usize) -> Fallible {
    put(buf, at, &u32::try_from(v)?.to_le_bytes())
}

/// A `VS_VERSIONINFO` node: header words, key, padding, value, padding,
/// children. Every value here is text or empty, so `wValueLength` counts
/// characters.
fn version_node(key: &str, value: &[u16], children: &[u16]) -> Fallible<Vec<u16>> {
    let mut node = vec![0, u16::try_from(value.len())?, 1];
    node.extend(key.encode_utf16());
    node.push(0);
    node.resize(node.len().next_multiple_of(2), 0);
    node.extend(value);
    node.resize(node.len().next_multiple_of(2), 0);
    node.extend(children);
    let length = u16::try_from(node.len() * 2)?;
    *node.first_mut().ok_or("no wLength")? = length;
    Ok(node)
}

/// A `VS_VERSIONINFO` blob holding the three fields in one string table.
fn version_blob_bytes() -> Fallible<Vec<u8>> {
    let mut strings: Vec<u16> = Vec::new();
    for (key, value) in [
        ("CompanyName", "Acme Games"),
        ("ProductName", "Rocket Sled"),
        ("OriginalFilename", "Sled.exe"),
    ] {
        let mut text: Vec<u16> = value.encode_utf16().collect();
        text.push(0);
        strings.resize(strings.len().next_multiple_of(2), 0);
        strings.extend(version_node(key, &text, &[])?);
    }
    let table = version_node("040904b0", &[], &strings)?;
    let info = version_node("StringFileInfo", &[], &table)?;
    let units = version_node("VS_VERSION_INFO", &[], &info)?;
    Ok(units.iter().flat_map(|u| u.to_le_bytes()).collect())
}

/// The `.rsrc` section's bytes, laid out for `SECTION_RVA`: a three-level
/// directory (`RT_VERSION`, name 1, language 0x409), the data entry, and
/// the blob.
fn rsrc_section() -> Fallible<Vec<u8>> {
    let blob = version_blob_bytes()?;
    let mut s = vec![0; 0x60 + blob.len()];
    // Each directory: 16-byte header, then one id entry.
    for (dir, id, child) in [
        (0x00, 16, 0x8000_0018),
        (0x18, 1, 0x8000_0030),
        (0x30, 0x409, 0x48),
    ] {
        put16(&mut s, dir + 14, 1)?;
        put32(&mut s, dir + 16, id)?;
        put32(&mut s, dir + 20, child)?;
    }
    put32(&mut s, 0x48, SECTION_RVA + 0x60)?;
    put32(&mut s, 0x4c, blob.len())?;
    put(&mut s, 0x60, &blob)?;
    Ok(s)
}

/// PE32 headers with one `.rsrc` section of `raw_size` bytes.
fn headers(raw_size: usize) -> Fallible<Vec<u8>> {
    let mut h = vec![0; SECTION_FILE_OFFSET];
    put(&mut h, 0, b"MZ")?;
    put32(&mut h, 0x3c, 0x40)?;
    put(&mut h, 0x40, b"PE\0\0")?;
    let coff = 0x44;
    put16(&mut h, coff, 0x14c)?;
    put16(&mut h, coff + 2, 1)?;
    put16(&mut h, coff + 16, 0xe0)?;
    let opt = coff + 20;
    put16(&mut h, opt, 0x10b)?;
    put32(&mut h, opt + 32, SECTION_ALIGNMENT)?;
    put32(&mut h, opt + 36, FILE_ALIGNMENT)?;
    put32(&mut h, opt + 56, SIZE_OF_IMAGE)?;
    put32(&mut h, opt + 60, SECTION_FILE_OFFSET)?;
    put32(&mut h, opt + 92, 16)?;
    put32(&mut h, opt + 96 + 16, SECTION_RVA)?;
    put32(&mut h, opt + 96 + 20, raw_size)?;
    let sec = opt + 0xe0;
    put(&mut h, sec, b".rsrc")?;
    put32(&mut h, sec + 8, raw_size)?;
    put32(&mut h, sec + 12, SECTION_RVA)?;
    put32(&mut h, sec + 16, raw_size)?;
    put32(&mut h, sec + 20, SECTION_FILE_OFFSET)?;
    Ok(h)
}

/// The image as it sits on disk: headers, then the section's raw bytes.
fn file_layout() -> Fallible<Vec<u8>> {
    let rsrc = rsrc_section()?;
    let mut file = headers(rsrc.len())?;
    file.extend(&rsrc);
    Ok(file)
}

/// The image as the loader maps it: the section at its RVA.
fn mapped_layout() -> Fallible<Vec<u8>> {
    let rsrc = rsrc_section()?;
    let mut image = vec![0; SIZE_OF_IMAGE];
    put(&mut image, 0, &headers(rsrc.len())?)?;
    put(&mut image, SECTION_RVA, &rsrc)?;
    Ok(image)
}

/// Lay a PE file out as the loader maps it, closely enough for the
/// resource walk: the headers at the start and each section's raw bytes
/// at its RVA, in a buffer of `SizeOfImage` bytes. Nothing is relocated
/// or bound.
fn map_file(file: &[u8]) -> Fallible<Vec<u8>> {
    let u16_at =
        |at: usize| -> Fallible<usize> { Ok(usize::from(file.u16(at).ok_or("file too short")?)) };
    let u32_at = |at: usize| -> Fallible<usize> {
        Ok(usize::try_from(file.u32(at).ok_or("file too short")?)?)
    };
    let coff = u32_at(0x3c)? + 4;
    let section_count = u16_at(coff + 2)?;
    let opt = coff + 20;
    let section_table = opt + u16_at(coff + 16)?;
    let mut image = vec![0; u32_at(opt + 56)?];
    let header_size = u32_at(opt + 60)?;
    put(
        &mut image,
        0,
        file.get(..header_size).ok_or("headers past the end")?,
    )?;
    for i in 0..section_count {
        let sec = section_table + i * 40;
        let virtual_size = u32_at(sec + 8)?;
        let rva = u32_at(sec + 12)?;
        let raw_size = u32_at(sec + 16)?;
        // The loader rounds PointerToRawData down to 0x200.
        let raw_offset = u32_at(sec + 20)? & !0x1ff;
        let len = if virtual_size == 0 {
            raw_size
        } else {
            raw_size.min(virtual_size)
        };
        let raw = file
            .get(raw_offset..raw_offset + len)
            .ok_or("section past the end")?;
        put(&mut image, rva, raw)?;
    }
    Ok(image)
}

fn expected() -> VersionInfo {
    VersionInfo {
        company: "Acme Games".into(),
        product: "Rocket Sled".into(),
        original_filename: "Sled.exe".into(),
    }
}

#[test]
fn the_mapped_image_is_read() -> Fallible {
    let image = mapped_layout()?;
    assert_eq!(read(image.as_ptr().cast()), expected());
    assert_eq!(from_image(image.as_slice()), expected());
    // Read at raw offsets, the resource RVA points past the end of the
    // file, which is why only a mapped image can be read.
    assert_eq!(
        from_image(file_layout()?.as_slice()),
        VersionInfo::default()
    );
    // The test's own loader lays the file out the same way.
    assert_eq!(map_file(&file_layout()?)?, image);
    Ok(())
}

#[test]
fn the_mapped_image_reports_its_machine() -> Fallible {
    let mut image = mapped_layout()?;
    assert_eq!(machine(image.as_ptr().cast()), Some(0x14c));
    put16(&mut image, 0x44, compatdb_core::IMAGE_FILE_MACHINE_ARM64)?;
    assert_eq!(
        machine(image.as_ptr().cast()),
        Some(compatdb_core::IMAGE_FILE_MACHINE_ARM64)
    );
    assert_eq!(machine(std::ptr::null()), None);
    // Without the PE signature there is no header to read it from.
    put(&mut image, 0x40, b"X")?;
    assert_eq!(machine(image.as_ptr().cast()), None);
    Ok(())
}

#[test]
fn a_foreign_or_missing_image_yields_nothing() -> Fallible {
    let mut not_pe = mapped_layout()?;
    put(&mut not_pe, 0x40, b"N")?;
    assert_eq!(read(not_pe.as_ptr().cast()), VersionInfo::default());
    assert_eq!(read(core::ptr::null()), VersionInfo::default());
    // A resource directory past SizeOfImage is not read.
    let mut short = mapped_layout()?;
    put32(&mut short, 0x44 + 20 + 56, SECTION_RVA)?;
    assert_eq!(read(short.as_ptr().cast()), VersionInfo::default());
    Ok(())
}

#[test]
fn a_wine_builtin_reads_when_one_is_installed() -> Fallible {
    // Not a fixture: the bundle in the dist/ tree next to this
    // repository, where the build scripts put it. A linked worktree has no
    // such tree, so set COMPATDB_TEST_PE to the path of a Wine
    // kernel32.dll (any PE whose OriginalFilename is kernel32.dll) to run
    // it there, for example
    // .../dist/wine/lib/wine/i386-windows/kernel32.dll. Skipped when the
    // file does not exist, with a line on stderr that cargo test only
    // shows when run with `-- --nocapture`.
    let path = std::env::var_os("COMPATDB_TEST_PE").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../dist/wine/lib/wine/i386-windows/kernel32.dll")
        },
        std::path::PathBuf::from,
    );
    if !path.is_file() {
        eprintln!(
            "skipping: no PE at {} (set COMPATDB_TEST_PE)",
            path.display()
        );
        return Ok(());
    }
    let image = map_file(&std::fs::read(&path)?)?;
    let version = read(image.as_ptr().cast());
    assert_eq!(
        version.original_filename.to_ascii_lowercase(),
        "kernel32.dll"
    );
    assert_ne!(version.company, "");
    Ok(())
}
