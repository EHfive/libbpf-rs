Unreleased
----------
- Removed `novendor` feature in favor of having disableable default
  feature
- Adjusted `SkeletonBuilder::clang_args` to accept an iterator of
  arguments instead of a string
- Added `--clang-args` argument to `make` and `build` sub-commands
- Put all generated types into single `<project>_types` module as opposed to
  having multiple modules for various sections (`.bss`, `.rodata`, etc.)
- Fixed potential naming issues by escaping reserved keywords used in
  identifiers
- Fixed potential unsoundness issues in generated skeletons by wrapping "unsafe"
  type in `MaybeUninit`
- Added pointer based ("raw") access to datasec type to generated skeletons
- Added better handling for bitfields to code generation logic
- Updated `libbpf-sys` dependency to `1.3.0`
- Bumped minimum Rust version to `1.71`


0.22.0
------
- Adjusted skeleton creation logic to generate shared and exclusive datasec
  accessor functions
- Removed `Error` enum in favor of `anyhow::Error`
- Bumped minimum Rust version to `1.65`


0.21.2
------
- Added `Default` impl for generated `struct` types containing pointers
- Fixed handling of function prototype type declaration inference in BTF and
  skeleton generation
- Improved error reporting in build script usage
- Bumped minimum Rust version to `1.64`


0.21.1
------
- Adjusted named padding members in generated types to have `pub` visibility


0.21.0
------
- Adjusted skeleton generation code to ensure implementation of `libbpf-rs`'s
  `SkelBuilder`, `OpenSkel`, and `Skel` traits
- Improved error reporting on BPF C file compilation failure


0.20.1
------
- Switched over to using `libbpf-rs`'s BTF support internally for skeleton
  generation
- Fixed potential build failures on systems defaulting to stack
  protector usage by passing `-fno-stack-protector` to `clang`


0.20.0
------
- Fixed mismatch in size of generated types with respect to corresponding C
  types
- Fixed generated skeleton potentially being unstable (changing each time)
- Implemented `Sync` for generated skeletons
- Made formatting using `rustfmt` optional
- Updated various dependencies


0.19.1
------
- Initial documented release
