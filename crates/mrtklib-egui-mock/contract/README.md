# Pinned contract

`openrpc.json` is a copy of MRTKLIB `docs/rpc/openrpc.json` (protocol 0.1) from
`hal1278/MRTKLIB`, branch `prototype/machine-interface`, commit `e66118b`.
The tests of this crate validate every message of the mock against it.

`openrpc.json` is part of MRTKLIB and is distributed under MRTKLIB's BSD
2-Clause license, not under this repository's license. `LICENSE` in this
directory is MRTKLIB's license file from the same commit, with its copyright
notices.

To follow a new version of the contract, replace the file, update the commit
here, copy MRTKLIB's `LICENSE` from the same commit, and change the mock until
`cargo test -p mrtklib-egui-mock` passes.
