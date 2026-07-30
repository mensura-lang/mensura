//! Byte-offset to LSP-position translation.
//!
//! Mensura spans are byte offsets into the source.  LSP positions are
//! `(line, character)` pairs whose `character` is counted in code units of the
//! negotiated [`PositionEncoding`].  A [`LineIndex`] precomputes line starts so
//! a byte offset maps to a position with one binary search plus a code-unit
//! count of the line prefix.

/// The unit `character` offsets are measured in, negotiated at `initialize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionEncoding {
    /// UTF-8 code units (bytes).  Preferred when the client supports it.
    Utf8,
    /// UTF-16 code units.  The protocol default.
    Utf16,
}

/// Length of `s` in code units of the given encoding.
pub fn encoded_len(s: &str, encoding: PositionEncoding) -> u32 {
    match encoding {
        PositionEncoding::Utf8 => s.len() as u32,
        PositionEncoding::Utf16 => s.chars().map(|c| c.len_utf16() as u32).sum(),
    }
}

/// The largest character boundary of `src` at or before `offset`, and never
/// past its end.  `str::floor_char_boundary` is unstable, so this is the walk.
fn clamp_to_boundary(src: &str, offset: usize) -> usize {
    let mut offset = offset.min(src.len());
    while !src.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The byte offset of the start of each line in a source string.
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut line_starts = vec![0];
        for (idx, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        LineIndex { line_starts }
    }

    /// The `(line, character)` of a byte offset, with `character` in the given
    /// encoding.  `line` and `character` are both zero-based, as LSP expects.
    ///
    /// An offset outside `src`, or inside a multi-byte character, is clamped
    /// rather than fatal: a language server must survive a span it cannot
    /// render.  Such a span is a bug elsewhere, and the clamped position sends
    /// the reader to the end of the file instead of killing the session.
    pub fn position(&self, src: &str, offset: usize, encoding: PositionEncoding) -> (u32, u32) {
        let offset = clamp_to_boundary(src, offset);
        // The line is the last line start at or before the offset.
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        let line_start = self.line_starts[line];
        let character = encoded_len(&src[line_start..offset], encoding);
        (line as u32, character)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_offsets_across_lines() {
        let src = "ab\ncde\n";
        let li = LineIndex::new(src);
        assert_eq!(li.position(src, 0, PositionEncoding::Utf8), (0, 0));
        assert_eq!(li.position(src, 2, PositionEncoding::Utf8), (0, 2));
        // First byte of the second line.
        assert_eq!(li.position(src, 3, PositionEncoding::Utf8), (1, 0));
        assert_eq!(li.position(src, 5, PositionEncoding::Utf8), (1, 2));
    }

    #[test]
    fn clamps_an_offset_from_outside_the_source() {
        // A diagnostic raised inside a bundled module carries an offset into
        // *its* source (spans have no file identity yet), which may run past
        // the document being rendered.  Clamp, never panic.
        let src = "ab\ncde\n";
        let li = LineIndex::new(src);
        assert_eq!(
            li.position(src, 1899, PositionEncoding::Utf8),
            li.position(src, src.len(), PositionEncoding::Utf8)
        );
        // Mid-character offsets walk back to the boundary.
        let multi = "é";
        let li = LineIndex::new(multi);
        assert_eq!(li.position(multi, 1, PositionEncoding::Utf8), (0, 0));
    }

    #[test]
    fn counts_multibyte_characters_per_encoding() {
        // `é` is two UTF-8 bytes and one UTF-16 unit; `𝄞` is four bytes and two
        // UTF-16 units.
        let src = "é𝄞x";
        let li = LineIndex::new(src);
        let x_offset = src.find('x').unwrap();
        assert_eq!(li.position(src, x_offset, PositionEncoding::Utf8), (0, 6));
        assert_eq!(li.position(src, x_offset, PositionEncoding::Utf16), (0, 3));
    }
}
