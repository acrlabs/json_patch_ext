//! This module provides some unofficial "extensions" to the [jsonpatch](https://jsonpatch.com)
//! format for describing changes to a JSON document.  In particular, it adds the `*` operator as a
//! valid token for arrays in a JSON document.  It means: apply this change to all elements of this
//! array.  For example, consider the following document:
//!
//! ```json
//! {
//!   "foo": {
//!     "bar": [
//!       {"baz": 1},
//!       {"baz": 2},
//!       {"baz": 3},
//!     ]
//!   }
//! }
//! ```
//!
//! The pathspec `/foo/bar/*/baz` would reference the `baz` field of all three array entries in the
//! `bar` array.  It is an error to use `*` to reference a field that is not an array.  It is an
//! error to use `*` at the end of a path, e.g., `/foo/*`.
//!
//! Additionally, this crate will auto-create parent paths for the AddOperation only, e.g., the
//! result of applying `AddOperation{ path: "/foo/bar", value: 1 }` to the empty document will be
//!
//! ```json
//! { "foo": {"bar": 1}}
//! ```

mod errors;
mod macros;

use json_patch::patch;
// mark these as re-exports in the generated docs (maybe related to
// https://github.com/rust-lang/rust/issues/131180?)
#[doc(no_inline)]
pub use json_patch::{
    AddOperation,
    CopyOperation,
    MoveOperation,
    Patch,
    PatchOperation,
    RemoveOperation,
    ReplaceOperation,
    TestOperation,
};
#[doc(no_inline)]
use jsonptr::index::Index;
use jsonptr::Token;
pub use jsonptr::{
    Pointer,
    PointerBuf,
};
use serde_json::{
    json,
    Value,
};

pub use crate::errors::PatchError;

// PatchMode controls what to do if the referenced element does not exist in the object.
#[derive(Debug, Clone, Copy)]
enum PatchMode {
    Error,
    Create,
    Skip,
}

pub fn add_operation(path: PointerBuf, value: Value) -> PatchOperation {
    PatchOperation::Add(AddOperation { path, value })
}

pub fn replace_operation(path: PointerBuf, value: Value) -> PatchOperation {
    PatchOperation::Replace(ReplaceOperation { path, value })
}

pub fn remove_operation(path: PointerBuf) -> PatchOperation {
    PatchOperation::Remove(RemoveOperation { path })
}

pub fn move_operation(from: PointerBuf, path: PointerBuf) -> PatchOperation {
    PatchOperation::Move(MoveOperation { from, path })
}

pub fn copy_operation(from: PointerBuf, path: PointerBuf) -> PatchOperation {
    PatchOperation::Copy(CopyOperation { from, path })
}

pub fn test_operation(path: PointerBuf, value: Value) -> PatchOperation {
    PatchOperation::Test(TestOperation { path, value })
}

pub fn escape(input: &str) -> String {
    Token::new(input).encoded().into()
}

pub fn patch_ext(obj: &mut Value, p: PatchOperation) -> Result<(), PatchError> {
    match p {
        PatchOperation::Add(op) => add_or_replace(obj, &op.path, &op.value, false)?,
        PatchOperation::Remove(op) => remove(obj, &op.path)?,
        PatchOperation::Replace(op) => add_or_replace(obj, &op.path, &op.value, true)?,
        x => patch(obj, &[x])?,
    }
    Ok(())
}

fn add_or_replace(obj: &mut Value, path: &PointerBuf, value: &Value, replace: bool) -> Result<(), PatchError> {
    let Some((subpath, tail)) = path.split_back() else {
        return Ok(());
    };

    // "replace" requires that the path you're replacing already exist, therefore we set
    // create_if_not_exists = !replace.  We don't want to skip missing elements.
    let mode = if replace { PatchMode::Error } else { PatchMode::Create };
    for v in patch_ext_helper(subpath, obj, mode)? {
        match v {
            Value::Object(map) => {
                let key = tail.decoded().into();
                if replace && !map.contains_key(&key) {
                    return Err(PatchError::TargetDoesNotExist(path.as_str().into()));
                }
                map.insert(key, value.clone());
            },
            Value::Array(vec) => match tail.to_index()? {
                Index::Num(idx) => {
                    vec.get(idx).ok_or(PatchError::OutOfBounds(idx))?;
                    if replace {
                        vec[idx] = value.clone();
                    } else {
                        vec.insert(idx, value.clone());
                    }
                },
                Index::Next => {
                    vec.push(value.clone());
                },
            },
            _ => {
                return Err(PatchError::UnexpectedType(path.as_str().into()));
            },
        }
    }

    Ok(())
}

fn remove(obj: &mut Value, path: &PointerBuf) -> Result<(), PatchError> {
    let Some((subpath, key)) = path.split_back() else {
        return Ok(());
    };

    for v in patch_ext_helper(subpath, obj, PatchMode::Skip)? {
        v.as_object_mut()
            .ok_or(PatchError::UnexpectedType(subpath.as_str().into()))?
            .remove(key.decoded().as_ref());
    }

    Ok(())
}

// Given JSON pointer, recursively walk through all the possible "end" values that the path
// references; return a mutable reference so we can make modifications at those points.
fn patch_ext_helper<'a>(
    path: &Pointer,
    value: &'a mut Value,
    mode: PatchMode,
) -> Result<Vec<&'a mut Value>, PatchError> {
    let Some(idx) = path.as_str().find("/*") else {
        if path.resolve(value).is_err() {
            match mode {
                PatchMode::Error => return Err(PatchError::TargetDoesNotExist(path.as_str().into())),
                PatchMode::Create => {
                    path.assign(value, json!({}))?;
                },
                PatchMode::Skip => return Ok(vec![]),
            }
        }
        return Ok(vec![path.resolve_mut(value)?]);
    };

    // we checked the index above so unwrap is safe here
    let (head, cons) = path.split_at(idx).unwrap();
    let mut res = vec![];

    // This is a little weird; if mode == Create, and the subpath up to this point doesn't exist,
    // we'll create an empty array which we won't iterate over at all.  I think that's
    // "approximately" fine and less surprising that not creating anything.
    if head.resolve(value).is_err() {
        match mode {
            PatchMode::Error => return Err(PatchError::TargetDoesNotExist(path.as_str().into())),
            PatchMode::Create => {
                path.assign(value, json!([]))?;
            },
            PatchMode::Skip => return Ok(vec![]),
        }
    }
    let next_array_val =
        head.resolve_mut(value)?.as_array_mut().ok_or(PatchError::UnexpectedType(head.as_str().into()))?;
    for v in next_array_val {
        if let Some((_, c)) = cons.split_front() {
            res.extend(patch_ext_helper(c, v, mode)?);
        } else {
            res.push(v);
        }
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use assertables::*;
    use rstest::*;

    use super::*;
    use crate as json_patch_ext; // make the macros work in the tests

    #[fixture]
    fn data() -> Value {
        json!({
            "foo": [
                {"baz": {"buzz": 0}},
                {"baz": {"quzz": 1}},
                {"baz": {"fixx": 2}},
            ],
        })
    }

    #[rstest]
    fn test_patch_ext_add(mut data: Value) {
        let path = format_ptr!("/foo/*/baz/buzz");
        let res = patch_ext(&mut data, add_operation(path, json!(42)));
        assert_ok!(res);
        assert_eq!(
            data,
            json!({
                "foo": [
                    {"baz": {"buzz": 42 }},
                    {"baz": {"quzz": 1, "buzz": 42}},
                    {"baz": {"fixx": 2, "buzz": 42}},
                ],
            })
        );
    }

    #[rstest]
    fn test_patch_ext_add_escaped() {
        let path = format_ptr!("/foo/bar/{}", escape("testing.sh/baz"));
        let mut data = json!({});
        let res = patch_ext(&mut data, add_operation(path, json!(42)));
        assert_ok!(res);
        assert_eq!(data, json!({"foo": {"bar": {"testing.sh/baz": 42}}}));
    }

    #[rstest]
    fn test_patch_ext_replace(mut data: Value) {
        let path = format_ptr!("/foo/*/baz");
        let res = patch_ext(&mut data, replace_operation(path, json!(42)));
        assert_ok!(res);
        assert_eq!(
            data,
            json!({
                "foo": [
                    {"baz": 42},
                    {"baz": 42},
                    {"baz": 42},
                ],
            })
        );
    }

    #[rstest]
    fn test_patch_ext_replace_err(mut data: Value) {
        let path = format_ptr!("/foo/*/baz/buzz");
        let res = patch_ext(&mut data, replace_operation(path, json!(42)));
        println!("{data:?}");
        assert_err!(res);
    }


    #[rstest]
    fn test_patch_ext_remove(mut data: Value) {
        let path = format_ptr!("/foo/*/baz/quzz");
        let res = patch_ext(&mut data, remove_operation(path));
        assert_ok!(res);
        assert_eq!(
            data,
            json!({
                "foo": [
                    {"baz": {"buzz": 0}},
                    {"baz": {}},
                    {"baz": {"fixx": 2}},
                ],
            })
        );
    }
}
