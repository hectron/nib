use jiff::tz::TimeZone;

pub(crate) fn named_fixed_zone(name: &str, offset_seconds: i32, abbreviation: [u8; 4]) -> TimeZone {
    let mut data = vec![0; 54];
    data[..4].copy_from_slice(b"TZif");
    data[36..40].copy_from_slice(&1_u32.to_be_bytes());
    data[40..44].copy_from_slice(&4_u32.to_be_bytes());
    data[44..48].copy_from_slice(&offset_seconds.to_be_bytes());
    data[50..].copy_from_slice(&abbreviation);
    TimeZone::tzif(name, &data).unwrap()
}
