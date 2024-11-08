# json-patch-ext

Unofficial extensions and helper functions for the [json-patch](https://github.com/idubrov/json-patch) crate.

## Features

* Support for the `*` operator when adding/replacing/removing elements: applies the operation to all elements in the
  array at that location in the path
* Some nice utility functions for constructing patches
* Automatically create parent references in an add operation: e.g., if your path is `/foo/bar/baz`, and your JSON object
  looks like `{"foo": {}}`, the result of the add operation will be `{"foo": {"bar": {"baz": <value>}}}`.
