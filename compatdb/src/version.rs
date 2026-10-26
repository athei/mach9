//! Reading a PE image's `VS_VERSIONINFO` so the database can match on a
//! version fingerprint (company, product, original filename) that is stable
//! across install location and patches.
//!
//! The image is the main image of the running process, which is already
//! mapped when this library loads, so a resource RVA is just an offset from
//! the image base. Every read is bounds checked against the image's own
//! extent: a malformed or resource-free image yields an empty
//! [`VersionInfo`], never a wild read.

use core::ffi::c_void;

use compatdb_core::VersionInfo;

/// Read the version fingerprint of the image mapped at `image_base`.
pub fn read(image_base: *const c_void) -> VersionInfo {
    Mapped::new(image_base).map_or_else(VersionInfo::default, |img| from_image(&img))
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
    version_blob(img)
        .map(|blob| compatdb_core::runtime::version_info(&blob))
        .unwrap_or_default()
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
