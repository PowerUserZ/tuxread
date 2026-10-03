// Copyright 2026 Google LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TimestampError;

/// Timestamps associated with an inode, relative time to EPOCH.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Timestamp {
    seconds: i64,
    nanoseconds: u32,
}

impl Timestamp {
    /// Seconds since EPOCH.
    #[must_use]
    pub fn seconds(self) -> i64 {
        self.seconds
    }

    /// Nanoseconds within the second.
    #[must_use]
    pub fn nanoseconds(self) -> u32 {
        self.nanoseconds
    }

    /// Decode a timestamp from the base field in the classic 128-byte
    /// inode, plus the corresponding `*_extra` field if present.
    pub(crate) fn from_raw(
        secs: u32,
        extra: Option<u32>,
    ) -> Result<Self, TimestampError> {
        // The kernel reads the base field as a signed 32-bit value: times
        // before 1970 (and, with the extra bits, after 2038) have the top bit set.
        let mut seconds = i64::from(secs.cast_signed());
        let mut nanoseconds = 0;

        if let Some(extra) = extra {
            // The lower two bits extend the seconds, the higher 30 bits hold
            // nanoseconds.
            seconds = seconds
                .checked_add(i64::from(extra & 0x3) << 32)
                .ok_or(TimestampError)?;
            nanoseconds = extra >> 2;

            // Ensure nanoseconds are valid.
            if nanoseconds >= 1_000_000_000 {
                return Err(TimestampError);
            }
        }

        Ok(Self {
            seconds,
            nanoseconds,
        })
    }
}

#[cfg(test)]
mod tuxread_patch_tests {
    use super::*;

    #[test]
    fn times_before_1970_and_after_2038_decode() {
        // 1969-01-01: the base field has its top bit set, no extra epoch bits.
        let t = Timestamp::from_raw((-31_536_000i32).cast_unsigned(), Some(0))
            .unwrap();
        assert_eq!((t.seconds(), t.nanoseconds()), (-31_536_000, 0));
        // 2^31 + 1000 s (2038-01-19): the base wraps negative, epoch bit 1 adds 2^32.
        let secs: i64 = (1 << 31) + 1000;
        let t = Timestamp::from_raw(secs as u32, Some(1)).unwrap();
        assert_eq!(t.seconds(), secs);
        // Classic 128-byte inode: no extra field, plain signed 32-bit seconds.
        assert_eq!(
            Timestamp::from_raw(0xFFFF_FFFF, None).unwrap().seconds(),
            -1
        );
    }
}
