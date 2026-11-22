//! Reading a PE image's `VS_VERSIONINFO` so the database can match on a
//! version fingerprint (company, product, original filename) that is stable
//! across install location and patches.
//!
//! For the main image of the running process, a resource RVA is an offset
//! from the mapped image base. For a pre-exec lookup it is translated to a
//! file offset through the child's PE section table.

use core::ffi::c_void;
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::{FileExt, OpenOptionsExt},
    path::Path,
};

use compatdb_core::VersionInfo;

/// Read the version fingerprint of the image mapped at `image_base`.
pub fn read(image_base: *const c_void) -> VersionInfo {
    Mapped::new(image_base).map_or_else(VersionInfo::default, |img| from_image(&img))
}

/// The same fingerprint as [`read`], but from an executable on disk before
/// the child has mapped it. Read only headers and the version resource.
pub fn read_file(path: &Path) -> Option<VersionInfo> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    file.metadata().ok()?.is_file().then_some(())?;
    let blob = version_blob(&OnDisk::new(file)?)?;
    let version = compatdb_core::runtime::version_info(&blob);
    (version != VersionInfo::default()).then_some(version)
}

impl Rva for File {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Option<()> {
        self.read_exact_at(buf, u64::try_from(offset).ok()?).ok()
    }
}

struct OnDisk {
    file: File,
    header_size: usize,
    sections: Vec<Section>,
}

struct Section {
    rva: usize,
    offset: usize,
    backed_size: usize,
}

impl OnDisk {
    fn new(file: File) -> Option<Self> {
        let pe = usize::try_from(file.u32(0x3c)?).ok()?;
        let mut signature = [0; 4];
        file.read(pe, &mut signature)?;
        if &signature != b"PE\0\0" {
            return None;
        }
        let coff = pe.checked_add(4)?;
        let count = usize::from(file.u16(coff.checked_add(2)?)?);
        // 96 is the PE loader's documented maximum.
        if count > 96 {
            return None;
        }
        let opt = coff.checked_add(20)?;
        let opt_size = usize::from(file.u16(coff.checked_add(16)?)?);
        let header_size = usize::try_from(file.u32(opt.checked_add(60)?)?).ok()?;
        let image_size = usize::try_from(file.u32(opt.checked_add(56)?)?).ok()?;
        let alignment = usize::try_from(file.u32(opt.checked_add(32)?)?).ok()?;
        let file_alignment = usize::try_from(file.u32(opt.checked_add(36)?)?).ok()?;
        if !alignment.is_power_of_two() || header_size > image_size {
            return None;
        }
        // Wine uses a flat file mapping for images with sub-page section alignment.
        let flat = alignment < 0x1000;
        if flat && alignment != file_alignment {
            return None;
        }
        let table = opt.checked_add(opt_size)?;
        if opt_size < 64 || table.checked_add(count * 40)? > header_size {
            return None;
        }
        let mut sections = Vec::with_capacity(count);
        for i in 0..count {
            let entry = table.checked_add(i * 40)?;
            let field = |n| usize::try_from(file.u32(entry.checked_add(n)?)?).ok();
            let virtual_size = field(8)?;
            let raw_size = field(16)?;
            let rva = field(12)?;
            let offset = field(20)?;
            if flat {
                if rva != offset {
                    return None;
                }
            } else if offset != 0 {
                // Wine's map_image_into_view rounds the raw file range to sectors
                // and caps it at the page-aligned extent of the mapped section.
                let mapped = if virtual_size == 0 {
                    raw_size
                } else {
                    virtual_size
                };
                let mapped = mapped.checked_next_multiple_of(alignment)?;
                let sector_offset = offset % 512;
                let backed = raw_size
                    .checked_add(sector_offset)?
                    .checked_next_multiple_of(512)?;
                sections.push(Section {
                    rva,
                    offset: offset - sector_offset,
                    backed_size: mapped.min(backed).min(image_size.checked_sub(rva)?),
                });
            }
        }
        Some(Self {
            file,
            header_size: if flat { image_size } else { header_size },
            sections,
        })
    }
}

impl Rva for OnDisk {
    fn read(&self, rva: usize, buf: &mut [u8]) -> Option<()> {
        if rva.checked_add(buf.len())? <= self.header_size {
            return self.file.read(rva, buf);
        }
        let offset = self.sections.iter().find_map(|section| {
            let start = rva.checked_sub(section.rva)?;
            if start.checked_add(buf.len())? > section.backed_size {
                return None;
            }
            section.offset.checked_add(start)
        })?;
        self.file.read(offset, buf)
    }
}

/// The COFF `Machine` of the image mapped at `image_base`, or `None` when its
/// headers cannot be read.
pub fn machine(image_base: *const c_void) -> Option<u16> {
    Mapped::new(image_base).map(|img| img.machine)
}

/// The most bytes read for a version resource. `VS_VERSIONINFO` records its
/// own length in a 16-bit field, so nothing past this can belong to it, and a
/// corrupt size field cannot make the reader allocate more.
const MAX_VERSION_BLOB: usize = 0x1_0000;

/// Bounds-checked reads by RVA from a PE image.
trait Rva {
    /// Fill `buf` with the bytes at `rva`, or return `None` when any of them
    /// lies outside what this source can supply.
    fn read(&self, rva: usize, buf: &mut [u8]) -> Option<()>;

    fn u16(&self, rva: usize) -> Option<u16> {
        let mut b = [0; 2];
        self.read(rva, &mut b)?;
        Some(u16::from_le_bytes(b))
    }

    fn u32(&self, rva: usize) -> Option<u32> {
        let mut b = [0; 4];
        self.read(rva, &mut b)?;
        Some(u32::from_le_bytes(b))
    }
}

/// The main image, mapped by the loader at `base` and `size` bytes long.
struct Mapped {
    base: *const u8,
    size: usize,
    /// The COFF `Machine` from the headers.
    machine: u16,
}

impl Mapped {
    /// The image at `image_base`, bounded by its own `SizeOfImage`.
    fn new(image_base: *const c_void) -> Option<Self> {
        if image_base.is_null() {
            return None;
        }
        let base = image_base.cast::<u8>();
        // The headers always occupy the first page, which is enough to read
        // the real extent from.
        let bootstrap = Self {
            base,
            size: 0x1000,
            machine: 0,
        };
        let headers = Headers::parse(&bootstrap)?;
        Some(Self {
            base,
            size: headers.size_of_image,
            machine: headers.machine,
        })
    }
}

impl Rva for Mapped {
    fn read(&self, rva: usize, buf: &mut [u8]) -> Option<()> {
        if rva.checked_add(buf.len())? > self.size {
            return None;
        }
        // SAFETY: rva..rva+len lies within [0, size), the image's own extent,
        // which the loader mapped readable before this library was loaded.
        let src = unsafe { std::slice::from_raw_parts(self.base.add(rva), buf.len()) };
        buf.copy_from_slice(src);
        Some(())
    }
}

/// The header fields the reader needs.
struct Headers {
    /// The COFF `Machine`.
    machine: u16,
    size_of_image: usize,
    /// The resource directory's RVA, 0 when the image has none.
    resource_rva: usize,
}

impl Headers {
    fn parse<R: Rva + ?Sized>(img: &R) -> Option<Self> {
        let e_lfanew = usize::try_from(img.u32(0x3c)?).ok()?;
        let mut signature = [0; 4];
        img.read(e_lfanew, &mut signature)?;
        if &signature != b"PE\0\0" {
            return None;
        }
        let coff = e_lfanew.checked_add(4)?;
        let machine = img.u16(coff)?;
        let opt = coff.checked_add(20)?;
        let pe32plus = img.u16(opt)? == 0x20b;
        let size_of_image = usize::try_from(img.u32(opt.checked_add(56)?)?).ok()?;
        // NumberOfRvaAndSizes, then the data directories (8 bytes each); the
        // resource directory is entry 2.
        let (count_at, dirs_at) = if pe32plus { (108, 112) } else { (92, 96) };
        let dir_count = img.u32(opt.checked_add(count_at)?)?;
        let resource_rva = if dir_count > 2 {
            usize::try_from(img.u32(opt.checked_add(dirs_at + 2 * 8)?)?).ok()?
        } else {
            0
        };
        Some(Self {
            machine,
            size_of_image,
            resource_rva,
        })
    }
}

fn from_image<R: Rva + ?Sized>(img: &R) -> VersionInfo {
    version_blob(img).map_or_default(|blob| compatdb_core::runtime::version_info(&blob))
}

fn version_blob<R: Rva + ?Sized>(img: &R) -> Option<Vec<u16>> {
    let res_rva = Headers::parse(img)?.resource_rva;
    if res_rva == 0 {
        return None;
    }

    // RT_VERSION, then the first name and the first language under it.
    let ver_dir = find_id_subdir(img, res_rva, res_rva, 16)?;
    let (name_child, name_is_dir) = first_child(img, res_rva, ver_dir)?;
    if !name_is_dir {
        return None;
    }
    let (lang_leaf, lang_is_dir) = first_child(img, res_rva, name_child)?;
    if lang_is_dir {
        return None;
    }

    // IMAGE_RESOURCE_DATA_ENTRY: OffsetToData (an RVA), Size.
    let data_rva = usize::try_from(img.u32(lang_leaf)?).ok()?;
    let data_size = usize::try_from(img.u32(lang_leaf.checked_add(4)?)?).ok()?;
    let mut raw = vec![0; data_size.min(MAX_VERSION_BLOB)];
    img.read(data_rva, &mut raw)?;

    // The blob is UTF-16; collect the u16 units.
    Some(
        raw.as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect(),
    )
}

/// Find the sub-directory for a resource entry with numeric id `id` in the
/// directory at `dir_off`. `res_base` is the resource section base (offsets in
/// entries are relative to it).
fn find_id_subdir<R: Rva + ?Sized>(
    img: &R,
    res_base: usize,
    dir_off: usize,
    id: u32,
) -> Option<usize> {
    let named = usize::from(img.u16(dir_off.checked_add(12)?)?);
    let ids = usize::from(img.u16(dir_off.checked_add(14)?)?);
    let entries = dir_off.checked_add(16)?;
    // The named entries come first, so only the id entries after them can
    // match. That also caps the scan at the 65535 entries the count can
    // declare, whatever a hostile name count says.
    for i in named..named.checked_add(ids)? {
        let eo = entries.checked_add(i.checked_mul(8)?)?;
        let name = img.u32(eo)?;
        let offset = img.u32(eo.checked_add(4)?)?;
        // An id entry whose id matches, pointing at a sub-directory (high bit
        // of offset set). An id entry has the high bit of its name clear, so
        // the comparison checks that too.
        if name == id && offset & 0x8000_0000 != 0 {
            return res_base.checked_add(usize::try_from(offset & 0x7fff_ffff).ok()?);
        }
    }
    None
}

/// The first entry of a resource directory: its child offset (relative to
/// `res_base`) and whether that child is another directory.
fn first_child<R: Rva + ?Sized>(img: &R, res_base: usize, dir_off: usize) -> Option<(usize, bool)> {
    let named = usize::from(img.u16(dir_off.checked_add(12)?)?);
    let ids = usize::from(img.u16(dir_off.checked_add(14)?)?);
    if named.checked_add(ids)? == 0 {
        return None;
    }
    let offset = img.u32(dir_off.checked_add(16 + 4)?)?;
    let child = res_base.checked_add(usize::try_from(offset & 0x7fff_ffff).ok()?)?;
    Some((child, offset & 0x8000_0000 != 0))
}

#[cfg(test)]
mod tests;
