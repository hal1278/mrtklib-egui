# Pinned contract

`openrpc.json` is a copy of MRTKLIB `docs/rpc/openrpc.json` (protocol 0.1) from
`hal1278/MRTKLIB`, branch `prototype/machine-interface`, commit `47cbc18`.
The tests of this crate validate every message of the mock against it.

To follow a new version of the contract, replace the file, update the commit
here, and change the mock until `cargo test -p mrtklib-egui-mock` passes.
