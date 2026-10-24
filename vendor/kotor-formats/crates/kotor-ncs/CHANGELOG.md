# Changelog

## Unreleased

### Features

* NCS bytecode reader and writer, plus a DeNCS-algorithm decompiler to NSS.
* The decompiler takes its engine-function signatures from a caller-supplied `ActionTable` (`ActionTable::from_nwscript` parses an install's `nwscript.nss`). No engine-function data is compiled in.
