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
pub use jsonptr::index::Index;
pub use jsonptr::{
    Pointer,
    PointerBuf,
    Token,
};
use serde_json::{
    Value,
    json,
};

pub use crate::errors::PatchError;

pub mod prelude {
    pub use super::{
        AddOperation,
        CopyOperation,
        MoveOperation,
        Patch,
        PatchError,
        PatchOperation,
        Pointer,
        PointerBuf,
        RemoveOperation,
        ReplaceOperation,
        TestOperation,
        Token,
        add_operation,
        copy_operation,
        escape,
        format_ptr,
        matches,
        move_operation,
        patch_ext,
        remove_operation,
        replace_operation,
        test_operation,
    };
}

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

pub fn copy_operation(from: PointerBuf, path: PointerBuf) -> PatchOperation {
    PatchOperation::Copy(CopyOperation { from, path })
}

pub fn move_operation(from: PointerBuf, path: PointerBuf) -> PatchOperation {
    PatchOperation::Move(MoveOperation { from, path })
}

pub fn remove_operation(path: PointerBuf) -> PatchOperation {
    PatchOperation::Remove(RemoveOperation { path })
}

pub fn replace_operation(path: PointerBuf, value: Value) -> PatchOperation {
    PatchOperation::Replace(ReplaceOperation { path, value })
}

pub fn test_operation(path: PointerBuf, value: Value) -> PatchOperation {
    PatchOperation::Test(TestOperation { path, value })
}

pub fn escape(input: &str) -> String {
    Token::new(input).encoded().into()
}

pub fn matches<'a>(path: &Pointer, value: &'a Value) -> Vec<(PointerBuf, &'a Value)> {
    let Some(idx) = path.as_str().find("/*") else {
        // Base case -- no stars;
        // If we can't resolve, there's no match to be found
        if let Ok(v) = path.resolve(value) {
            return vec![(path.to_buf(), v)];
        } else {
            return vec![];
        }
    };

    // we checked the index above so unwrap is safe here
    let (head, cons) = path.split_at(idx).unwrap();
    let mut res = vec![];

    // If we can't resolve the head, or it's not an array, no match found
    let Ok(head_val) = head.resolve(value) else {
        return vec![];
    };
    let Some(next_array_val) = head_val.as_array() else {
        return vec![];
    };

    for (i, v) in next_array_val.iter().enumerate() {
        // /1 is a valid pointer so the unwrap below is fine
        let idx_str = format!("/{i}");
        let idx_path = PointerBuf::parse(&idx_str).unwrap();

        // The cons pointer either looks like /* or /*/something, so we need to split_front
        // to get the array marker out, and either return the current path if there's nothing
        // else, or recurse and concatenate the subpath(s) to the head
        if let Some((_, c)) = cons.split_front() {
            let subpaths = matches(c, v);
            res.extend(subpaths.iter().map(|(p, v)| (head.concat(&idx_path.concat(p)), *v)));
        } else {
            unreachable!("cons can't be root");
        }
    }
    res
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

fn add_or_replace(obj: &mut Value, path: &Pointer, value: &Value, replace: bool) -> Result<(), PatchError> {
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
                    return Err(PatchError::TargetDoesNotExist(path.to_string()));
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
                return Err(PatchError::UnexpectedType(path.to_string()));
            },
        }
    }

    Ok(())
}

fn remove(obj: &mut Value, path: &Pointer) -> Result<(), PatchError> {
    let Some((subpath, key)) = path.split_back() else {
        *obj = Value::Null;
        return Ok(());
    };

    for v in patch_ext_helper(subpath, obj, PatchMode::Skip)? {
        match v {
            Value::Object(map) => {
                map.remove(key.decoded().as_ref());
            },
            Value::Array(vec) => {
                if key.decoded() == "*" {
                    vec.clear();
                } else if let Index::Num(idx) = key.to_index()? {
                    vec.get(idx).ok_or(PatchError::OutOfBounds(idx))?;
                    vec.remove(idx);
                } else {
                    return Err(PatchError::UnexpectedType(key.to_string()));
                }
            },
            _ => {
                return Err(PatchError::UnexpectedType(path.to_string()));
            },
        }
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

    // Head now points at what we believe is an array; if not, it's an error.
    let next_array_val =
        head.resolve_mut(value)?.as_array_mut().ok_or(PatchError::UnexpectedType(head.as_str().into()))?;

    // Iterate over all the array values and recurse, returning all found values
    for v in next_array_val {
        // The cons pointer either looks like /* or /*/something, so we need to split_front
        // to get the array marker out, and either return the current value if there's nothing
        // else, or recurse and return all the found values
        if let Some((_, c)) = cons.split_front() {
            res.extend(patch_ext_helper(c, v, mode)?);
        } else {
            unreachable!("cons can't be root");
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
    fn test_matches_1(data: Value) {
        let path = format_ptr!("/foo");
        let m: Vec<_> = matches(&path, &data).iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(m, vec![format_ptr!("/foo")]);
    }

    #[rstest]
    fn test_matches_2(data: Value) {
        let path = format_ptr!("/foo/*/baz");
        let m: Vec<_> = matches(&path, &data).iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(m, vec![format_ptr!("/foo/0/baz"), format_ptr!("/foo/1/baz"), format_ptr!("/foo/2/baz")]);
    }

    #[rstest]
    fn test_matches_3(data: Value) {
        let path = format_ptr!("/foo/*");
        let m: Vec<_> = matches(&path, &data).iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(m, vec![format_ptr!("/foo/0"), format_ptr!("/foo/1"), format_ptr!("/foo/2")]);
    }

    #[rstest]
    #[case(format_ptr!("/foo/*/baz/fixx"))]
    #[case(format_ptr!("/foo/2/baz/fixx"))]
    fn test_matches_4(#[case] path: PointerBuf, data: Value) {
        let m: Vec<_> = matches(&path, &data).iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(m, vec![format_ptr!("/foo/2/baz/fixx")]);
    }

    #[rstest]
    fn test_matches_root() {
        let path = format_ptr!("/*");
        let data = json!(["foo", "bar"]);
        let m: Vec<_> = matches(&path, &data).iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(m, vec![format_ptr!("/0"), format_ptr!("/1")]);
    }

    #[rstest]
    #[case(format_ptr!("/*"))]
    #[case(format_ptr!("/food"))]
    #[case(format_ptr!("/foo/3/baz"))]
    #[case(format_ptr!("/foo/bar/baz"))]
    #[case(format_ptr!("/foo/0/baz/fixx"))]
    fn test_no_match(#[case] path: PointerBuf, data: Value) {
        let m = matches(&path, &data);
        assert_is_empty!(m);
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
    fn test_patch_ext_add_vec1(mut data: Value) {
        let path = format_ptr!("/foo/1");
        let res = patch_ext(&mut data, add_operation(path, json!(42)));
        assert_ok!(res);
        assert_eq!(
            data,
            json!({
                "foo": [
                    {"baz": {"buzz": 0}},
                    42,
                    {"baz": {"quzz": 1}},
                    {"baz": {"fixx": 2}},
                ],
            })
        );
    }

    #[rstest]
    fn test_patch_ext_add_vec2(mut data: Value) {
        let path = format_ptr!("/foo/-");
        let res = patch_ext(&mut data, add_operation(path, json!(42)));
        assert_ok!(res);
        assert_eq!(
            data,
            json!({
                "foo": [
                    {"baz": {"buzz": 0}},
                    {"baz": {"quzz": 1}},
                    {"baz": {"fixx": 2}},
                    42,
                ],
            })
        );
    }

    #[rstest]
    fn test_patch_ext_add_vec_err(mut data: Value) {
        let path = format_ptr!("/foo/a");
        let res = patch_ext(&mut data, add_operation(path, json!(42)));
        assert_err!(res);
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
    fn test_patch_ext_replace_vec1(mut data: Value) {
        let path = format_ptr!("/foo/1");
        let res = patch_ext(&mut data, replace_operation(path, json!(42)));
        assert_ok!(res);
        assert_eq!(
            data,
            json!({
                "foo": [
                    {"baz": {"buzz": 0}},
                    42,
                    {"baz": {"fixx": 2}},
                ],
            })
        );
    }

    #[rstest]
    fn test_patch_ext_replace_vec2(mut data: Value) {
        let path = format_ptr!("/foo/-");
        let res = patch_ext(&mut data, replace_operation(path, json!(42)));
        assert_ok!(res);
        assert_eq!(
            data,
            json!({
                "foo": [
                    {"baz": {"buzz": 0}},
                    {"baz": {"quzz": 1}},
                    {"baz": {"fixx": 2}},
                    42,
                ],
            })
        );
    }

    #[rstest]
    fn test_patch_ext_replace_err(mut data: Value) {
        let path = format_ptr!("/foo/*/baz/buzz");
        let res = patch_ext(&mut data, replace_operation(path, json!(42)));
        assert_err!(res);
    }

    #[rstest]
    fn test_patch_ext_remove_root(mut data: Value) {
        let path = format_ptr!("");
        let res = patch_ext(&mut data, remove_operation(path));
        assert_ok!(res);
        assert_eq!(data, json!(null));
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

    #[rstest]
    fn test_patch_ext_remove_wildcard(mut data: Value) {
        let path = format_ptr!("/foo/*");
        let res = patch_ext(&mut data, remove_operation(path));
        assert_ok!(res);
        assert_eq!(data, json!({"foo": []}));
    }

    #[rstest]
    fn test_patch_ext_remove_nested_wildcards() {
        let mut data = json!([[
            [{"bar": "baz"}, {"qwerty": "yuiop"}],
            [{"fuzz": "quzz"}],
            [{"asdf": "hjkl"}],
        ]]);
        let path = format_ptr!("/*/*/*");
        let res = patch_ext(&mut data, remove_operation(path));
        assert_ok!(res);
        assert_eq!(data, json!([[[], [], []]]));
    }

    #[rstest]
    fn test_patch_ext_remove_vec(mut data: Value) {
        let path = format_ptr!("/foo/1");
        let res = patch_ext(&mut data, remove_operation(path));
        assert_ok!(res);
        assert_eq!(
            data,
            json!({
                "foo": [
                    {"baz": {"buzz": 0}},
                    {"baz": {"fixx": 2}},
                ],
            })
        );
    }

    #[rstest]
    fn test_patch_ext_remove_vec_err(mut data: Value) {
        let path = format_ptr!("/foo/-");
        let res = patch_ext(&mut data, remove_operation(path));
        assert_err!(res);
    }
}
