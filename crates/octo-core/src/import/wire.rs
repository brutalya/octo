//! Minimal protobuf wire-format reader: just enough to walk the schema-less
//! `steps.step_payload` blobs of the Antigravity CLI. No schema, no allocation of
//! nested messages; every malformed input is an error, never a panic.

/// One field value; only the wire types the reader needs to see.
#[derive(Debug, Clone, Copy)]
pub enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    /// Fixed 32/64-bit values are skipped over but not interpreted.
    Fixed,
}

pub struct Fields<'a> {
    buf: &'a [u8],
}

pub fn fields(buf: &[u8]) -> Fields<'_> {
    Fields { buf }
}

fn varint(buf: &mut &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for (i, b) in buf.iter().enumerate().take(10) {
        v |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            *buf = &buf[i + 1..];
            return Some(v);
        }
    }
    None
}

fn take<'a>(buf: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (head, tail) = buf.split_at_checked(n)?;
    *buf = tail;
    Some(head)
}

/// Yields `Ok((field_number, value))`; a malformed tail yields one `Err` and ends.
impl<'a> Iterator for Fields<'a> {
    type Item = Result<(u64, Value<'a>), ()>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        let item = (|| {
            let key = varint(&mut self.buf)?;
            let value = match key & 7 {
                0 => Value::Varint(varint(&mut self.buf)?),
                1 => {
                    take(&mut self.buf, 8)?;
                    Value::Fixed
                }
                2 => {
                    let n = usize::try_from(varint(&mut self.buf)?).ok()?;
                    Value::Bytes(take(&mut self.buf, n)?)
                }
                5 => {
                    take(&mut self.buf, 4)?;
                    Value::Fixed
                }
                _ => return None,
            };
            Some((key >> 3, value))
        })();
        match item {
            Some(i) => Some(Ok(i)),
            None => {
                self.buf = &[];
                Some(Err(()))
            }
        }
    }
}

/// First length-delimited field `n` of a message, or `None` if absent or malformed.
pub fn bytes_field(buf: &[u8], n: u64) -> Option<&[u8]> {
    fields(buf)
        .map_while(Result::ok)
        .find_map(|(f, v)| match v {
            Value::Bytes(b) if f == n => Some(b),
            _ => None,
        })
}

/// First varint field `n` of a message.
pub fn varint_field(buf: &[u8], n: u64) -> Option<u64> {
    fields(buf)
        .map_while(Result::ok)
        .find_map(|(f, v)| match v {
            Value::Varint(x) if f == n => Some(x),
            _ => None,
        })
}
