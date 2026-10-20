# boxed-builders

Small runnable example for storage-core's static and boxed service builders.

Run it with:

```sh
cargo run --manifest-path examples/boxed-builders/Cargo.toml
```

The example builds the same mock file-object service four ways:

- `FileObjectBuilder::new().layer(...).backend(backend)` keeps the full static
  layer stack. This is the default zero-dispatch path.
- `FileObjectBuilder::new().layer(...).boxed(backend)` builds the static stack
  and erases the final service behind `Arc<dyn FileObject>`.
- `BoxFileObjectBuilder::new(backend).layer(...).build()` erases after each
  layer, which can reduce compile-time type pressure for deep stacks.
- `BoxFileObjectFileFolderBuilder::new(backend).layer(...).build()` does the
  same for layers that need a service implementing both `FileObject` and
  `FileFolder`, such as list-cache-style layers.

All four forms preserve source-order request flow: the first layer written is
the first layer to observe the request.
