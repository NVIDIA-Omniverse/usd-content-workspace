//! Recovery of significant whitespace in XML text nodes that the serde deserializer trims.
//!
//! `azure_core::xml::read_xml` deserializes via quick-xml's serde
//! [`Deserializer`](quick_xml::de::Deserializer), which trims leading and trailing ASCII
//! whitespace from every text node. That silently corrupts values such as blob names that
//! legitimately contain leading or trailing spaces (e.g. `"my file "` is returned by Azure as
//! `<Name>my file </Name>` but deserializes to `"my file"`). Listing operations re-read the raw
//! response bytes with this module to restore the exact values after deserialization.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// Recover the untrimmed text content of every `<child>` element whose *direct parent* element
/// name is one of `parents`, in document order.
///
/// This re-reads `body` with a low-level [`Reader`] (trimming disabled) so callers can restore
/// the exact text after a [`read_xml`](azure_core::xml::read_xml) round-trip, matching values up
/// by document order.
///
/// Values whose start tag carries an `Encoded="true"` attribute — as the List Blobs API sets for
/// names containing characters invalid in XML (REST version 2021-02-12 and later) — are
/// percent-decoded per RFC 2396.
///
/// The direct-parent filter ensures unrelated `<child>` elements are ignored. For example a user
/// metadata key that happens to be named `Name` (`<Metadata><Name>...</Name></Metadata>`) is not
/// mistaken for a blob name, because its parent is `Metadata` rather than `Blob`.
pub(crate) fn recover_untrimmed_text(body: &[u8], parents: &[&str], child: &str) -> Vec<String> {
    let child = child.as_bytes();
    let parents: Vec<&[u8]> = parents.iter().map(|p| p.as_bytes()).collect();
    let is_target = |stack: &[Vec<u8>]| {
        stack
            .last()
            .is_some_and(|parent| parents.contains(&parent.as_slice()))
    };

    let mut reader = Reader::from_reader(body);
    reader.trim_text(false);
    reader.expand_empty_elements(false);

    let mut values = Vec::new();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut buf = Vec::new();

    let mut capturing = false;
    let mut encoded = false;
    let mut current = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_vec();
                if name == child && is_target(&stack) {
                    capturing = true;
                    encoded = e.attributes().flatten().any(|attr| {
                        attr.key.as_ref() == b"Encoded" && attr.value.as_ref() == b"true"
                    });
                    current.clear();
                }
                stack.push(name);
            }
            // A self-closing `<Name/>` is an empty value; keep the index aligned with the
            // serde-parsed collection by recording an empty string.
            Ok(Event::Empty(e)) => {
                if e.name().as_ref() == child && is_target(&stack) {
                    values.push(String::new());
                }
            }
            Ok(Event::Text(t)) if capturing => {
                if let Ok(unescaped) = t.unescape() {
                    current.push_str(&unescaped);
                }
            }
            Ok(Event::CData(t)) if capturing => {
                current.push_str(&String::from_utf8_lossy(&t.into_inner()));
            }
            Ok(Event::End(_)) => {
                let popped = stack.pop();
                if capturing && popped.as_deref() == Some(child) {
                    capturing = false;
                    let value = if encoded {
                        percent_decode_rfc2396(&current)
                    } else {
                        current.clone()
                    };
                    values.push(value);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    values
}

/// Decode an RFC 2396 percent-encoded string (used for List Blobs `Encoded="true"` names).
///
/// Invalid or truncated escape sequences are passed through verbatim, and any resulting non-UTF-8
/// bytes are replaced with U+FFFD, so this never fails.
fn percent_decode_rfc2396(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_whitespace_and_filters_by_parent() {
        // A `<Name>` whose parent is `Metadata` (not `Blob`/`BlobPrefix`) must be ignored, and
        // `Encoded="true"` names must be percent-decoded.
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<EnumerationResults>
  <Blobs>
    <Blob>
      <Name>rustam-test/bin/Material </Name>
      <Properties><Content-Length>19</Content-Length></Properties>
      <Metadata><Name>i-am-a-metadata-key</Name></Metadata>
    </Blob>
    <Blob>
      <Name> leading-and-trailing </Name>
      <Properties><Content-Length>1</Content-Length></Properties>
    </Blob>
    <BlobPrefix>
      <Name>some-folder/ </Name>
    </BlobPrefix>
    <Blob>
      <Name Encoded="true">weird%20%EF%BF%BEname</Name>
      <Properties><Content-Length>0</Content-Length></Properties>
    </Blob>
  </Blobs>
  <NextMarker />
</EnumerationResults>"#;

        let names = recover_untrimmed_text(xml, &["Blob", "BlobPrefix"], "Name");
        assert_eq!(
            names,
            vec![
                "rustam-test/bin/Material ".to_string(),
                " leading-and-trailing ".to_string(),
                "some-folder/ ".to_string(),
                "weird \u{fffe}name".to_string(),
            ]
        );
    }

    #[test]
    fn unescapes_entities() {
        let xml = br#"<Blobs><Blob><Name>a &amp; b </Name></Blob></Blobs>"#;
        let names = recover_untrimmed_text(xml, &["Blob"], "Name");
        assert_eq!(names, vec!["a & b ".to_string()]);
    }

    #[test]
    fn handles_self_closing() {
        let xml = br#"<Blobs><Blob><Name/></Blob><Blob><Name>x </Name></Blob></Blobs>"#;
        let names = recover_untrimmed_text(xml, &["Blob"], "Name");
        assert_eq!(names, vec![String::new(), "x ".to_string()]);
    }

    #[test]
    fn percent_decode_passthrough_on_invalid() {
        assert_eq!(percent_decode_rfc2396("100%"), "100%");
        assert_eq!(percent_decode_rfc2396("a%2"), "a%2");
        assert_eq!(percent_decode_rfc2396("a%zzb"), "a%zzb");
        assert_eq!(percent_decode_rfc2396("a%20b"), "a b");
    }
}
