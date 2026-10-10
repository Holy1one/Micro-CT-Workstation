//! Strict PTP card-storage datasets and offline writable-card planning.
//!
//! This is the parsing/planning foundation for D7100 SD-card capture. No device
//! is contacted here: every dataset is caller-supplied bytes from a future
//! authorized session. Parsers therefore fail closed on truncation, trailing
//! bytes, reserved or duplicate identifiers, unknown access capabilities and
//! inconsistent capacity evidence instead of guessing a value the camera never
//! reported. Card-resident object metadata is validated by
//! `ptp::parse_card_object_info`, which shares none of the RAM tolerance.

use super::ptp::{invalid, Cursor};
use super::types::CameraError;
use serde::Serialize;

/// Bounded `GetStorageIDs` cardinality. A camera advertising more than this is
/// treated as malformed rather than trusted with unbounded allocation.
const MAX_STORAGE_IDS: usize = 64;
/// Bounded `GetObjectHandles` cardinality for one card listing.
const MAX_OBJECT_HANDLES: usize = 100_000;
/// PTP's reserved "all storages" id, never a real storage.
const RESERVED_STORAGE_ID: u32 = 0xffff_ffff;

/// A camera-advertised storage id from PTP `GetStorageIDs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct StorageId(pub u32);

/// PTP `AccessCapability`. Object deletion is not capture, so capability 2
/// ("read-only with object deletion") remains read-only for planning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum StorageAccess {
    ReadWrite,
    ReadOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct StorageCapacity {
    pub max_bytes: u64,
    pub free_bytes: u64,
    pub free_images: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StorageInfo {
    pub id: StorageId,
    pub access: StorageAccess,
    pub capacity: StorageCapacity,
    pub storage_type: u16,
    pub filesystem_type: u16,
    pub description: String,
    pub volume_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CardPlan {
    pub storages: Vec<StorageInfo>,
    pub writable_bytes: u64,
    pub writable_images: u32,
}

/// PTP `GetStorageIDs`: UINT32 count followed by exactly that many UINT32 ids,
/// little endian, with no trailing bytes. `count == 0` is valid evidence that a
/// camera reports no storage; the caller decides what that means.
pub fn parse_storage_ids(bytes: &[u8]) -> Result<Vec<StorageId>, CameraError> {
    let mut c = Cursor::new(bytes);
    let count = c.u32()? as usize;
    if count > MAX_STORAGE_IDS {
        return Err(invalid("PTP storage id count exceeds limit"));
    }
    let mut ids: Vec<StorageId> = Vec::with_capacity(count);
    for _ in 0..count {
        let id = StorageId(c.u32()?);
        if id.0 == RESERVED_STORAGE_ID {
            return Err(invalid("PTP storage id is the reserved 0xffffffff value"));
        }
        if ids.contains(&id) {
            return Err(invalid("duplicate PTP storage id"));
        }
        ids.push(id);
    }
    c.finish()?;
    Ok(ids)
}

/// PTP `GetStorageInfo` dataset: StorageType u16, FilesystemType u16,
/// AccessCapability u16, MaxCapacity u64, FreeSpaceInBytes u64,
/// FreeSpaceInImages u32, StorageDescription (PTP string), VolumeLabel (PTP
/// string), then end of dataset.
///
/// The dataset carries no StorageID field, so the returned [`StorageInfo::id`]
/// is the neutral [`StorageId(0)`] placeholder. `0` is not a reserved storage
/// id; a caller enumerating ids binds the real one from `parse_storage_ids`
/// rather than this parser inventing it.
pub fn parse_storage_info(bytes: &[u8]) -> Result<StorageInfo, CameraError> {
    let mut c = Cursor::new(bytes);
    let storage_type = c.u16()?;
    let filesystem_type = c.u16()?;
    let access = match c.u16()? {
        0 => StorageAccess::ReadWrite,
        1 | 2 => StorageAccess::ReadOnly,
        _ => return Err(invalid("unknown PTP storage access capability")),
    };
    let max_bytes = u64::from_le_bytes(c.take(8)?.try_into().unwrap());
    let free_bytes = u64::from_le_bytes(c.take(8)?.try_into().unwrap());
    let free_images = c.u32()?;
    let description = c.string()?;
    let volume_label = c.string()?;
    c.finish()?;
    if free_bytes > max_bytes {
        return Err(invalid("PTP storage free space exceeds its max capacity"));
    }
    Ok(StorageInfo {
        id: StorageId(0),
        access,
        capacity: StorageCapacity {
            max_bytes,
            free_bytes,
            free_images,
        },
        storage_type,
        filesystem_type,
        description,
        volume_label,
    })
}

/// PTP `GetObjectHandles` for one storage: UINT32 count followed by exactly that
/// many UINT32 handles, no trailing bytes, no duplicates.
pub fn parse_object_handles(bytes: &[u8]) -> Result<Vec<u32>, CameraError> {
    let mut c = Cursor::new(bytes);
    let count = c.u32()? as usize;
    if count > MAX_OBJECT_HANDLES {
        return Err(invalid("PTP object handle count exceeds limit"));
    }
    let mut handles: Vec<u32> = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let handle = c.u32()?;
        if handles.contains(&handle) {
            return Err(invalid("duplicate PTP object handle"));
        }
        handles.push(handle);
    }
    c.finish()?;
    Ok(handles)
}

/// Plans card storage for host-to-card capture using only writable storage in
/// advertised order. Stable uppercase error codes:
/// `CARD_READ_ONLY` (nothing writable), `CARD_CAPACITY_UNKNOWN` (a writable
/// storage reports zero max capacity), `CARD_CAPACITY_INSUFFICIENT`
/// (writable totals below the request), `CARD_CAPACITY_OVERFLOW` (checked
/// arithmetic would wrap). A single sufficient card is a complete plan; a
/// missing second slot is not an error.
pub fn plan_cards(
    infos: &[StorageInfo],
    min_bytes: u64,
    min_images: u32,
) -> Result<CardPlan, String> {
    let storages: Vec<StorageInfo> = infos
        .iter()
        .filter(|info| info.access == StorageAccess::ReadWrite)
        .cloned()
        .collect();
    if storages.is_empty() {
        return Err("CARD_READ_ONLY".into());
    }
    let mut writable_bytes: u64 = 0;
    let mut writable_images: u32 = 0;
    for storage in &storages {
        if storage.capacity.max_bytes == 0 {
            return Err("CARD_CAPACITY_UNKNOWN".into());
        }
        writable_bytes = writable_bytes
            .checked_add(storage.capacity.max_bytes)
            .ok_or("CARD_CAPACITY_OVERFLOW")?;
        writable_images = writable_images
            .checked_add(storage.capacity.free_images)
            .ok_or("CARD_CAPACITY_OVERFLOW")?;
    }
    if writable_bytes < min_bytes || writable_images < min_images {
        return Err("CARD_CAPACITY_INSUFFICIENT".into());
    }
    Ok(CardPlan {
        storages,
        writable_bytes,
        writable_images,
    })
}

/// Conservative per-frame byte upper bound for a D7100 14-bit lossless DX NEF.
/// The Nikon manual's DX 14-bit lossless example is about 28.5 MB; this bound keeps
/// headroom for a busier scene instead of pretending to know the real file size.
pub const CONSERVATIVE_NEF_BYTES: u64 = 64 * 1024 * 1024;

/// Frames one scan writes to the card: 10 pre-dark + 10 pre-flat references plus
/// every projection. A card plan must cover all of them, not only the projections.
pub fn task_card_frames(projection_count: u32) -> u32 {
    projection_count.saturating_add(20)
}

/// Validates that the writable card capacity covers a whole task at the
/// conservative per-frame bound. Capacity is checked before any device action, and
/// the totals use [`plan_cards`]' checked arithmetic, so a missing second slot is
/// never assumed to contribute capacity.
pub fn plan_task_cards(
    infos: &[StorageInfo],
    projection_count: u32,
    per_frame_bytes: u64,
) -> Result<CardPlan, String> {
    if projection_count == 0 {
        return Err("CARD_TASK_EMPTY".into());
    }
    if per_frame_bytes == 0 {
        return Err("CARD_FRAME_BOUND_UNKNOWN".into());
    }
    let frames = task_card_frames(projection_count);
    let required = u64::from(frames)
        .checked_mul(per_frame_bytes)
        .ok_or("CARD_CAPACITY_OVERFLOW")?;
    plan_cards(infos, required, frames)
}

/// Binds camera-advertised storage ids to parsed `GetStorageInfo` datasets in the
/// order the camera returned them. A PTP `GetStorageInfo` dataset carries no
/// StorageID field, so the only proven binding is positional; a count mismatch is
/// refused instead of pairing an unproven id with a card. Use this before
/// [`plan_cards`] so a receipt names the storage that actually received an
/// exposure rather than the neutral placeholder.
pub fn bind_storage_ids(
    ids: &[StorageId],
    infos: Vec<StorageInfo>,
) -> Result<Vec<StorageInfo>, CameraError> {
    if ids.len() != infos.len() {
        return Err(invalid("PTP storage id and storage info counts differ"));
    }
    Ok(infos
        .into_iter()
        .zip(ids.iter().copied())
        .map(|(mut info, id)| {
            info.id = id;
            info
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::super::ptp::{parse_card_object_info, MAX_OBJECT_BYTES};
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    /// PTP string: UINT8 count of UTF-16 units including the NUL terminator.
    fn ptp_string(text: &str) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let mut b = vec![units.len() as u8];
        b.extend(units.into_iter().flat_map(u16::to_le_bytes));
        b
    }

    /// GetStorageInfo dataset with the fields in their declared PTP order.
    fn storage_info_bytes(
        storage_type: u16,
        filesystem_type: u16,
        access: u16,
        max_bytes: u64,
        free_bytes: u64,
        free_images: u32,
        description: &str,
        label: &str,
    ) -> Vec<u8> {
        let mut b = storage_type.to_le_bytes().to_vec();
        b.extend(filesystem_type.to_le_bytes());
        b.extend(access.to_le_bytes());
        b.extend(max_bytes.to_le_bytes());
        b.extend(free_bytes.to_le_bytes());
        b.extend(free_images.to_le_bytes());
        b.extend(ptp_string(description));
        b.extend(ptp_string(label));
        b
    }

    fn writable(id: u32, max_bytes: u64, free_images: u32) -> StorageInfo {
        StorageInfo {
            id: StorageId(id),
            access: StorageAccess::ReadWrite,
            capacity: StorageCapacity {
                max_bytes,
                free_bytes: max_bytes,
                free_images,
            },
            storage_type: 0x0004,
            filesystem_type: 0x0002,
            description: "SD card".into(),
            volume_label: "MICROCT".into(),
        }
    }

    fn read_only(id: u32, max_bytes: u64, free_images: u32) -> StorageInfo {
        StorageInfo {
            access: StorageAccess::ReadOnly,
            ..writable(id, max_bytes, free_images)
        }
    }

    /// D7100 ObjectInfo date string: UINT8 unit count, UTF-16LE units, NUL
    /// terminated with optional trailing NUL padding.
    fn d7100_date(text: &str) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let mut b = vec![units.len() as u8];
        b.extend(units.into_iter().flat_map(u16::to_le_bytes));
        b
    }

    /// Card ObjectInfo dataset in declared field order. The 40-byte numeric
    /// block is ThumbFormat u16, ThumbCompressedSize u32, ThumbPixWidth u32,
    /// ThumbPixHeight u32, ImagePixWidth u32, ImagePixHeight u32,
    /// ImageBitDepth u32, ParentObject u32, AssociationType u16,
    /// AssociationDesc u32, SequenceNumber u32.
    fn card_object_bytes(
        storage_id: u32,
        format: u16,
        protection: u16,
        size: u32,
        parent: u32,
        filename: &str,
        capture_date: &str,
    ) -> Vec<u8> {
        let mut b = storage_id.to_le_bytes().to_vec();
        b.extend(format.to_le_bytes());
        b.extend(protection.to_le_bytes());
        b.extend(size.to_le_bytes());
        b.extend([0x08, 0x38]); // ThumbFormat 0x3808
        b.extend(7791u32.to_le_bytes()); // ThumbCompressedSize
        b.extend(160u32.to_le_bytes()); // ThumbPixWidth
        b.extend(120u32.to_le_bytes()); // ThumbPixHeight
        b.extend(6000u32.to_le_bytes()); // ImagePixWidth
        b.extend(4000u32.to_le_bytes()); // ImagePixHeight
        b.extend(14u32.to_le_bytes()); // ImageBitDepth
        b.extend(parent.to_le_bytes()); // ParentObject
        b.extend(0u16.to_le_bytes()); // AssociationType
        b.extend(0u32.to_le_bytes()); // AssociationDesc
        b.extend(0u32.to_le_bytes()); // SequenceNumber
        b.extend(ptp_string(filename));
        b.extend(d7100_date(capture_date));
        b.extend(d7100_date(capture_date)); // ModificationDate
        b.extend(ptp_string("")); // Keywords
        b
    }

    fn card_object(size: u32) -> Vec<u8> {
        card_object_bytes(
            0x0001_0001,
            0x3800,
            0,
            size,
            0x0002_0005,
            "DSC_0001.NEF",
            "20161004T224936",
        )
    }

    #[test]
    fn storage_ids_parse_exactly_the_declared_little_endian_ids() {
        let bytes = [
            0x02, 0x00, 0x00, 0x00, // count 2
            0x01, 0x00, 0x01, 0x00, // 0x00010001
            0x02, 0x00, 0x00, 0x00, // 0x00000002
        ];
        assert_eq!(
            parse_storage_ids(&bytes).unwrap(),
            vec![StorageId(0x0001_0001), StorageId(0x0000_0002)]
        );
    }

    #[test]
    fn storage_ids_reject_reserved_duplicate_malformed_and_unbounded_payloads() {
        // Reserved 0xFFFFFFFF is not a storage.
        assert!(parse_storage_ids(&[
            0x01, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff
        ])
        .is_err());
        // Duplicates prove the listing is inconsistent.
        assert!(parse_storage_ids(&[
            0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00
        ])
        .is_err());
        // Declared two ids, supplied one.
        assert!(parse_storage_ids(&[
            0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00
        ])
        .is_err());
        // Truncated count itself.
        assert!(parse_storage_ids(&[0x01, 0x00]).is_err());
        // One trailing byte after a complete listing.
        assert!(parse_storage_ids(&[
            0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00
        ])
        .is_err());
        // Count above the bound is rejected before reading any id.
        let mut over = 65u32.to_le_bytes().to_vec();
        over.extend([0u8; 4 * 65]);
        assert!(parse_storage_ids(&over).is_err());
    }

    #[test]
    fn empty_storage_listing_is_valid_evidence_of_no_storage() {
        assert_eq!(
            parse_storage_ids(&[0x00, 0x00, 0x00, 0x00]).unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn storage_info_parses_the_declared_field_order_and_keeps_64_bit_capacity() {
        let mut bytes = vec![
            0x04, 0x00, // StorageType 0x0004 (removable RAM)
            0x02, 0x00, // FilesystemType 0x0002 (DCIM)
            0x01, 0x00, // AccessCapability 1 = read-only
        ];
        // MaxCapacity 64 GiB = 0x0000001000000000
        bytes.extend([0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00]);
        // FreeSpaceInBytes 32 GiB = 0x0000000800000000
        bytes.extend([0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00]);
        // FreeSpaceInImages 4000 = 0x00000FA0
        bytes.extend([0xa0, 0x0f, 0x00, 0x00]);
        bytes.extend(ptp_string("D7100 SD"));
        bytes.extend(ptp_string("MICROCT"));
        let info = parse_storage_info(&bytes).unwrap();
        assert_eq!(info.access, StorageAccess::ReadOnly);
        assert_eq!(info.storage_type, 0x0004);
        assert_eq!(info.filesystem_type, 0x0002);
        assert_eq!(info.capacity.max_bytes, 64 * GIB);
        assert_eq!(info.capacity.free_bytes, 32 * GIB);
        assert_eq!(info.capacity.free_images, 4000);
        assert_eq!(info.description, "D7100 SD");
        assert_eq!(info.volume_label, "MICROCT");
        // GetStorageInfo carries no id; the parser reports the neutral value
        // instead of inventing an id the dataset never contained.
        assert_eq!(info.id, StorageId(0));
    }

    #[test]
    fn storage_info_maps_access_capabilities_and_rejects_unknown_values() {
        assert_eq!(
            parse_storage_info(&storage_info_bytes(3, 2, 0, GIB, 0, 0, "ram", "")).unwrap().access,
            StorageAccess::ReadWrite
        );
        for capability in [1u16, 2] {
            assert_eq!(
                parse_storage_info(&storage_info_bytes(3, 2, capability, GIB, 0, 0, "", ""))
                    .unwrap()
                    .access,
                StorageAccess::ReadOnly
            );
        }
        for capability in [3u16, 4, 0xffff] {
            assert!(parse_storage_info(&storage_info_bytes(3, 2, capability, GIB, 0, 0, "", ""))
                .is_err());
        }
    }

    #[test]
    fn storage_info_rejects_inconsistent_free_space_truncation_and_trailing_bytes() {
        // Free bytes above the reported capacity is inconsistent evidence.
        assert!(parse_storage_info(&storage_info_bytes(3, 2, 0, GIB, GIB + 1, 1, "", "")).is_err());
        let valid = storage_info_bytes(3, 2, 0, 2 * GIB, GIB, 12, "card", "MICROCT");
        let mut truncated = valid.clone();
        truncated.pop();
        assert!(parse_storage_info(&truncated).is_err());
        let mut short = valid.clone();
        short.truncate(23); // inside MaxCapacity
        assert!(parse_storage_info(&short).is_err());
        let mut trailing = valid;
        trailing.push(0x00);
        assert!(parse_storage_info(&trailing).is_err());
    }

    #[test]
    fn object_handles_parse_and_reject_count_mismatch_duplicates_and_over_limit() {
        let bytes = [
            0x03, 0x00, 0x00, 0x00, // count 3
            0x01, 0x00, 0x0b, 0x00, // 0x000b0001
            0x02, 0x00, 0x0b, 0x00, // 0x000b0002
            0x03, 0x00, 0x0b, 0x00, // 0x000b0003
        ];
        assert_eq!(
            parse_object_handles(&bytes).unwrap(),
            vec![0x000b_0001, 0x000b_0002, 0x000b_0003]
        );
        // Declared three handles, supplied two.
        assert!(parse_object_handles(&bytes[..12]).is_err());
        // Trailing byte after a complete listing.
        let mut trailing = bytes.to_vec();
        trailing.push(0x00);
        assert!(parse_object_handles(&trailing).is_err());
        // Duplicate handle.
        assert!(parse_object_handles(&[
            0x02, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00
        ])
        .is_err());
        // Count above the bound is rejected before reading any handle.
        let mut over = 100_001u32.to_le_bytes().to_vec();
        over.extend([0u8; 16]);
        assert!(parse_object_handles(&over).is_err());
        assert_eq!(
            parse_object_handles(&[0x00, 0x00, 0x00, 0x00]).unwrap(),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn plan_sums_two_writable_64_gib_cards_without_truncation() {
        let plan = plan_cards(
            &[
                writable(0x0001_0001, 64 * GIB, 4000),
                writable(0x0001_0002, 64 * GIB, 4200),
            ],
            1,
            1,
        )
        .unwrap();
        assert_eq!(plan.writable_bytes, 128 * GIB);
        assert_eq!(plan.writable_images, 8200);
        assert_eq!(plan.storages.len(), 2);
        assert_eq!(plan.storages[0].id, StorageId(0x0001_0001));
    }

    #[test]
    fn plan_accepts_one_sufficient_card_with_a_read_only_or_absent_second_slot() {
        let single = plan_cards(&[writable(0x0001_0001, 64 * GIB, 9000)], 32 * GIB, 100).unwrap();
        assert_eq!(single.writable_bytes, 64 * GIB);
        assert_eq!(single.storages.len(), 1);
        let with_read_only = plan_cards(
            &[
                writable(0x0001_0001, 64 * GIB, 9000),
                read_only(0x0001_0002, 8 * GIB, 10),
            ],
            32 * GIB,
            100,
        )
        .unwrap();
        assert_eq!(with_read_only.writable_bytes, 64 * GIB);
        assert_eq!(with_read_only.writable_images, 9000);
        assert_eq!(with_read_only.storages.len(), 1);
        assert_eq!(with_read_only.storages[0].access, StorageAccess::ReadWrite);
    }

    #[test]
    fn plan_reports_read_only_unknown_and_insufficient_capacity() {
        assert_eq!(plan_cards(&[], 1, 1).unwrap_err(), "CARD_READ_ONLY");
        assert_eq!(
            plan_cards(&[read_only(0x0001_0001, 64 * GIB, 9000)], 1, 1).unwrap_err(),
            "CARD_READ_ONLY"
        );
        assert_eq!(
            plan_cards(&[writable(0x0001_0001, 0, 0)], 1, 1).unwrap_err(),
            "CARD_CAPACITY_UNKNOWN"
        );
        // A second unknown-capacity card cannot be hidden by a sufficient first
        // card: the total is not proven.
        assert_eq!(
            plan_cards(
                &[writable(0x0001_0001, 64 * GIB, 9000), writable(0x0001_0002, 0, 0)],
                1,
                1
            )
            .unwrap_err(),
            "CARD_CAPACITY_UNKNOWN"
        );
        assert_eq!(
            plan_cards(&[writable(0x0001_0001, 32 * GIB, 9000)], 64 * GIB, 1).unwrap_err(),
            "CARD_CAPACITY_INSUFFICIENT"
        );
        // Stale free-image evidence below the request.
        assert_eq!(
            plan_cards(&[writable(0x0001_0001, 64 * GIB, 3)], 1, 10).unwrap_err(),
            "CARD_CAPACITY_INSUFFICIENT"
        );
    }

    #[test]
    fn plan_reports_overflow_instead_of_wrapping_totals() {
        assert_eq!(
            plan_cards(
                &[
                    writable(0x0001_0001, u64::MAX, 1),
                    writable(0x0001_0002, u64::MAX, 1)
                ],
                1,
                1
            )
            .unwrap_err(),
            "CARD_CAPACITY_OVERFLOW"
        );
        assert_eq!(
            plan_cards(
                &[
                    writable(0x0001_0001, 64 * GIB, u32::MAX),
                    writable(0x0001_0002, 64 * GIB, u32::MAX)
                ],
                1,
                1
            )
            .unwrap_err(),
            "CARD_CAPACITY_OVERFLOW"
        );
    }

    #[test]
    fn card_object_accepts_bounded_nef_metadata_for_declared_formats() {
        for format in [0x3000u16, 0xb101, 0x3800] {
            let bytes =
                card_object_bytes(0x0001_0001, format, 0, 26_673_241, 5, "DSC_0001.NEF", "20161004T224936");
            let object = parse_card_object_info(&bytes).unwrap();
            assert_eq!(object.storage_id, 0x0001_0001);
            assert_eq!(object.parent, 5);
            assert_eq!(object.format, format);
            assert_eq!(object.protection, 0);
            assert_eq!(object.size, 26_673_241);
            assert_eq!(object.filename, "DSC_0001.NEF");
            assert_eq!(object.capture_date, "20161004T224936");
        }
        // Case-insensitive NEF suffix and both inclusive size bounds.
        for size in [8u32, MAX_OBJECT_BYTES as u32] {
            let bytes =
                card_object_bytes(0x0001_0001, 0x3800, 0, size, 0, "dsc_0002.nef", "20161004T224936");
            assert_eq!(parse_card_object_info(&bytes).unwrap().size, size as usize);
        }
    }

    #[test]
    fn card_object_rejects_reserved_storage_ids_and_unsafe_names() {
        for storage in [0u32, 0xffff_ffff] {
            let bytes =
                card_object_bytes(storage, 0x3800, 0, 1024, 0, "DSC_0001.NEF", "20161004T224936");
            assert!(parse_card_object_info(&bytes).is_err());
        }
        // Same header, only the filename varies, so each rejection is chargeable
        // to the name rather than to another field.
        let header = card_object(1024)[..52].to_vec();
        let named = |name: &str| {
            let mut rebuilt = header.clone();
            rebuilt.extend(ptp_string(name));
            rebuilt.extend(d7100_date("20161004T224936"));
            rebuilt.extend(d7100_date("20161004T224936"));
            rebuilt.extend(ptp_string(""));
            rebuilt
        };
        assert!(parse_card_object_info(&named("DSC_0001.NEF")).is_ok());
        for name in [
            "",
            "DSC_0001.JPG",
            "DSC_0001.NEF ",
            "DSC_0001.NEFF",
            "DSC_0001",
            "/DSC_0001.NEF",
            "DCIM\\DSC_0001.NEF",
            "..\\DSC_0001.NEF",
            "DSC_0001\n.NEF",
            "DSC_\u{7f}0001.NEF",
            "DSC_0001\0.NEF",
        ] {
            assert!(
                parse_card_object_info(&named(name)).is_err(),
                "unsafe card filename accepted: {name:?}"
            );
        }
        // A PTP string's UINT8 unit count caps a representable name at 254
        // characters, so the stricter 255-character bound is defensive; the
        // longest encodable name must still be accepted.
        let longest = format!("{}.NEF", "A".repeat(250));
        assert_eq!(longest.chars().count(), 254);
        assert!(parse_card_object_info(&named(&longest)).is_ok());
    }

    #[test]
    fn card_object_rejects_out_of_range_size_unknown_format_truncation_and_trailing_bytes() {
        for size in [0u32, 7, MAX_OBJECT_BYTES as u32 + 1, u32::MAX] {
            assert!(
                parse_card_object_info(&card_object(size)).is_err(),
                "out-of-range card object size accepted: {size}"
            );
        }
        for format in [0x0000u16, 0x3001, 0x3801, 0xb100, 0xffff] {
            assert!(parse_card_object_info(&card_object_bytes(
                0x0001_0001,
                format,
                0,
                1024,
                0,
                "DSC_0001.NEF",
                "20161004T224936"
            ))
            .is_err());
        }
        let valid = card_object(1024);
        let mut truncated = valid.clone();
        truncated.pop();
        assert!(parse_card_object_info(&truncated).is_err());
        let mut short = valid.clone();
        short.truncate(51); // inside the numeric block
        assert!(parse_card_object_info(&short).is_err());
        let mut trailing = valid;
        trailing.push(0x00);
        assert!(parse_card_object_info(&trailing).is_err());
    }

    fn bound_info(access: StorageAccess) -> StorageInfo {
        StorageInfo {
            id: StorageId(0),
            access,
            capacity: StorageCapacity {
                max_bytes: 64 * GIB,
                free_bytes: 32 * GIB,
                free_images: 3000,
            },
            storage_type: 0,
            filesystem_type: 0,
            description: String::new(),
            volume_label: String::new(),
        }
    }

    #[test]
    fn advertised_ids_bind_positionally_and_a_count_mismatch_is_refused() {
        let ids = [StorageId(0x0001_0001), StorageId(0x0001_0002)];
        let bound = bind_storage_ids(
            &ids,
            vec![
                bound_info(StorageAccess::ReadWrite),
                bound_info(StorageAccess::ReadOnly),
            ],
        )
        .unwrap();
        assert_eq!(bound[0].id, StorageId(0x0001_0001));
        assert_eq!(bound[1].id, StorageId(0x0001_0002));
        assert_eq!(bound[1].access, StorageAccess::ReadOnly);
        // Too few or too many datasets cannot be paired with an unproven id.
        assert!(bind_storage_ids(&ids, vec![bound_info(StorageAccess::ReadWrite)]).is_err());
        assert!(bind_storage_ids(&[], vec![bound_info(StorageAccess::ReadWrite)]).is_err());
        // Binding keeps the writable planning order of the camera.
        let planned = plan_cards(&bound, 64 * GIB, 3000).unwrap();
        assert_eq!(planned.storages.len(), 1);
        assert_eq!(planned.storages[0].id, StorageId(0x0001_0001));
    }

    fn sized_info(max_bytes: u64, free_images: u32, access: StorageAccess) -> StorageInfo {
        StorageInfo {
            id: StorageId(1),
            access,
            capacity: StorageCapacity {
                max_bytes,
                free_bytes: max_bytes,
                free_images,
            },
            storage_type: 0,
            filesystem_type: 0,
            description: String::new(),
            volume_label: String::new(),
        }
    }

    #[test]
    fn whole_task_card_plan_covers_references_and_projections_or_refuses() {
        // 10 pre-dark + 10 pre-flat references plus every projection.
        assert_eq!(task_card_frames(360), 380);
        let single = [sized_info(64 * GIB, 5000, StorageAccess::ReadWrite)];
        let plan = plan_task_cards(&single, 360, CONSERVATIVE_NEF_BYTES).unwrap();
        assert_eq!(plan.storages.len(), 1);
        assert!(plan.writable_bytes >= 380 * CONSERVATIVE_NEF_BYTES);
        assert!(plan.writable_images >= 380);
        // Two cards plan as two writable storages in camera order.
        let dual = [
            sized_info(64 * GIB, 5000, StorageAccess::ReadWrite),
            sized_info(64 * GIB, 5000, StorageAccess::ReadWrite),
        ];
        assert_eq!(
            plan_task_cards(&dual, 360, CONSERVATIVE_NEF_BYTES)
                .unwrap()
                .storages
                .len(),
            2
        );
        // A card that cannot cover the whole task refuses before any device action.
        assert_eq!(
            plan_task_cards(
                &[sized_info(16 * GIB, 500, StorageAccess::ReadWrite)],
                360,
                CONSERVATIVE_NEF_BYTES
            )
            .unwrap_err(),
            "CARD_CAPACITY_INSUFFICIENT"
        );
        // Read-only and unknown-capacity evidence keep their own codes.
        assert_eq!(
            plan_task_cards(
                &[sized_info(64 * GIB, 5000, StorageAccess::ReadOnly)],
                360,
                CONSERVATIVE_NEF_BYTES
            )
            .unwrap_err(),
            "CARD_READ_ONLY"
        );
        assert_eq!(
            plan_task_cards(
                &[sized_info(0, 0, StorageAccess::ReadWrite)],
                360,
                CONSERVATIVE_NEF_BYTES
            )
            .unwrap_err(),
            "CARD_CAPACITY_UNKNOWN"
        );
        // The free-image count is part of the evidence, not only the byte capacity.
        assert_eq!(
            plan_task_cards(
                &[sized_info(64 * GIB, 10, StorageAccess::ReadWrite)],
                360,
                CONSERVATIVE_NEF_BYTES
            )
            .unwrap_err(),
            "CARD_CAPACITY_INSUFFICIENT"
        );
        // Degenerate inputs are refused instead of defaulting a bound.
        assert_eq!(
            plan_task_cards(&single, 0, CONSERVATIVE_NEF_BYTES).unwrap_err(),
            "CARD_TASK_EMPTY"
        );
        assert_eq!(
            plan_task_cards(&single, 360, 0).unwrap_err(),
            "CARD_FRAME_BOUND_UNKNOWN"
        );
        // A huge bound cannot wrap the required total into a passing value.
        assert_eq!(
            plan_task_cards(&single, 360, u64::MAX).unwrap_err(),
            "CARD_CAPACITY_OVERFLOW"
        );
    }
}
