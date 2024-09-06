<h1 align="center" style="border-bottom: none">
    <b>Eepy OS</b>
</h1>
<p align="center" style="border-bottom: solid 1px; padding-bottom: 15px">An (incomplete) kernel in Rust for the Star64</p>

## Build and Deploy

The kernel is built as an executable ELF with
```
cargo build --release
```
It assumes the OpenSBI firmware is available, and has a memory footprint fully contained in the first megabyte of memory, from `0x40000000` to `0x40100000` and claims all the remaining memory, assuming 4GB of RAM (total).

Currently, no method of booting the kernel is provided. Boot the ELF on a Star64 using any method, as long as OpenSBI is available.
