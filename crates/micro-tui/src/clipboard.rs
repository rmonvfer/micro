//! Reading and writing the system clipboard.

use std::io::Read as _;
use std::io::Write as _;
use std::process::Command;
use std::process::Stdio;

/// An image taken off the clipboard, ready to attach to a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub mime_type: String,
    /// The image itself, base64 encoded, which is the shape every provider wants.
    pub data: String,
}

/// The image types worth asking for, best first.
const IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Put text on the clipboard, reporting whether anything was there to do it.
pub fn write_text(text: &str) -> bool {
    for (program, arguments) in [
        ("pbcopy", &[][..]),
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
    ] {
        let Ok(mut child) = Command::new(program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().map(|status| status.success()).unwrap_or(false) {
            return true;
        }
    }
    false
}

/// Take an image off the clipboard, if one is there.
pub fn read_image() -> Option<ClipboardImage> {
    read_via_wayland()
        .or_else(read_via_x11)
        .or_else(read_via_macos)
}

fn read_via_wayland() -> Option<ClipboardImage> {
    let listed = run(&["wl-paste", "--list-types"])?;
    let available = String::from_utf8_lossy(&listed);
    let mime_type = IMAGE_TYPES
        .iter()
        .find(|candidate| available.lines().any(|line| line.trim() == **candidate))?;
    let data = run(&["wl-paste", "--type", mime_type, "--no-newline"])?;
    encoded(mime_type, &data)
}

fn read_via_x11() -> Option<ClipboardImage> {
    let listed = run(&["xclip", "-selection", "clipboard", "-t", "TARGETS", "-o"])?;
    let available = String::from_utf8_lossy(&listed);
    let mime_type = IMAGE_TYPES
        .iter()
        .find(|candidate| available.lines().any(|line| line.trim() == **candidate))?;
    let data = run(&["xclip", "-selection", "clipboard", "-t", mime_type, "-o"])?;
    encoded(mime_type, &data)
}

/// macOS has no way to list what it holds.
fn read_via_macos() -> Option<ClipboardImage> {
    let path = std::env::temp_dir().join(format!("micro-clipboard-{}.png", std::process::id()));
    let script = format!(
        "set f to (open for access POSIX file \"{}\" with write permission)\n\
         try\n\
             write (the clipboard as «class PNGf») to f\n\
             close access f\n\
         on error\n\
             close access f\n\
             error\n\
         end try",
        path.display()
    );

    let ran = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;

    let image = match ran.success() {
        true => std::fs::read(&path)
            .ok()
            .and_then(|bytes| match bytes.is_empty() {
                true => None,
                false => encoded("image/png", &bytes),
            }),
        false => None,
    };
    let _ = std::fs::remove_file(&path);
    image
}

fn encoded(mime_type: &str, bytes: &[u8]) -> Option<ClipboardImage> {
    if bytes.is_empty() {
        return None;
    }
    Some(ClipboardImage {
        mime_type: mime_type.to_string(),
        data: base64(bytes),
    })
}

/// Run a command and hand back its output, or nothing when it is absent or fails.
fn run(command: &[&str]) -> Option<Vec<u8>> {
    let (program, arguments) = command.split_first()?;
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut output = Vec::new();
    if let Some(stdout) = child.stdout.as_mut() {
        stdout.read_to_end(&mut output).ok()?;
    }
    match child.wait().ok()?.success() && !output.is_empty() {
        true => Some(output),
        false => None,
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let block = match chunk.len() {
            1 => (chunk[0] as u32) << 16,
            2 => ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8),
            _ => ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | chunk[2] as u32,
        };
        out.push(ALPHABET[(block >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(block >> 12 & 63) as usize] as char);

        out.push(match chunk.len() > 1 {
            true => ALPHABET[(block >> 6 & 63) as usize] as char,
            false => '=',
        });
        out.push(match chunk.len() > 2 {
            true => ALPHABET[(block & 63) as usize] as char,
            false => '=',
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_bytes_outside_ascii() {
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
        assert_eq!(base64(&[0x00, 0x00, 0x00]), "AAAA");
    }

    #[test]
    fn encoding_nothing_yields_no_image() {
        assert!(encoded("image/png", &[]).is_none());
    }

    #[test]
    fn an_encoded_image_carries_its_type() {
        let image = encoded("image/png", b"foobar").expect("an image");
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.data, "Zm9vYmFy");
    }
}
