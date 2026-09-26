//! CBOR because `Decimal` needs a self-describing format.

use super::ast::{Module, Node};
use crate::common::error::LoadError;
use crate::common::limits::{MAX_BLOB_BYTES, MAX_TREE_DEPTH};

const SIGNATURE: [u8; 4] = *b"TMPL";

/// Version of the blob format [`Template::to_bytes`](crate::Template::to_bytes)
/// writes.
pub const BLOB_VERSION: u32 = 1;

pub(crate) fn encode(module: &Module) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&SIGNATURE);
    out.extend_from_slice(&BLOB_VERSION.to_le_bytes());
    ciborium::ser::into_writer(module, &mut out)
        .expect("encoding a compiled module to CBOR cannot fail");
    out
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Module, LoadError> {
    if bytes.len() > MAX_BLOB_BYTES {
        return Err(LoadError::Corrupt("blob exceeds maximum size".into()));
    }
    let header = bytes
        .get(..8)
        .ok_or_else(|| LoadError::Corrupt("blob too short".into()))?;
    if header[..4] != SIGNATURE {
        return Err(LoadError::Corrupt("not a Temple blob".into()));
    }
    let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if version != BLOB_VERSION {
        return Err(LoadError::IncompatibleVersion {
            found: version,
            expected: BLOB_VERSION,
        });
    }
    // ciborium's default recursion limit (256 levels) stays, as in 0.3.0: the
    // deepest trees need about 860, but decoding overflows a 2 MB debug stack
    // near 700
    let module: Module = ciborium::de::from_reader(&bytes[8..])
        .map_err(|e| LoadError::Corrupt(format!("malformed blob: {e}")))?;
    if too_deep(&module) {
        return Err(LoadError::Corrupt("blob nests too deeply".into()));
    }
    Ok(module)
}

/// Walked without recursion: nothing else may walk the tree before this passes.
fn too_deep(module: &Module) -> bool {
    let lets = module.lets.iter().map(|binding| Node::Expr(&binding.expr));
    let mut stack: Vec<(Node, usize)> = lets
        .chain([Node::Out(&module.output)])
        .map(|node| (node, 1))
        .collect();
    while let Some((node, depth)) = stack.pop() {
        if depth > MAX_TREE_DEPTH {
            return true;
        }
        node.for_each_child(&mut |child| stack.push((child, depth + 1)));
    }
    false
}
