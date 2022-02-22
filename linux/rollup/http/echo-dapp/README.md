# Dapp example echo application

This is an example of a decentralized application for the Cartesi emulator. It implements the interface defined [here](https://github.com/cartesi/rollups/blob/develop/openapi/dapp.yaml).
Developers can study this codebase to familiarize themselves with how a rollup interaction loop works.
It is also a good starting point for those who want to create their own rust Dapps. A good way to start is to extend `Model.rs`.

To communicate with the client outside of emulator, `/dev/rollup` linux device is used. Http dispatcher application translates requests read from Linux device to http `advance/inspect` requests and passes them to the Dapp. Dapp communicates the results of its execution to the outside world by using `voucher/notice/report/finish` API of the http dispatcher.

## Getting Started

This project requires Rust.
To install Rust follow the instructions [here](https://www.rust-lang.org/tools/install).


## Build for risc-v cpu

Prerequisites: `riscv64-cartesi-linux-gnu` toolchain must be in the $PATH

### Build
```shell
$ source ./environment.sh
$ cargo +nightly build -Z build-std=std,core,alloc,panic_abort,proc_macro --target riscv64ima-cartesi-linux-gnu.json --release
```

## Execution 

Utility script `rollup-init`  is used to start simultaneously Dapp application and http dispatcher service (available in `linux/utils` folder of the root `machine-emulator-tools` project).

After build copy Dapp and http dispatcher application to the new directory. Create ext2 filesystem image from that directory.

```shell
$ genext2fs -b 14k -d echo/ echo.ext2
```

Run application with `cartesi-machine` emulator:
```shell
$ cartesi-machine-server --server-address=127.0.0.1:10001
```
In other terminal send advance request:
```shell
$  cartesi-machine --rollup --server-address=127.0.0.1:10001 --checkin-address=127.0.0.1:10003 --flash-drive=label:hello,filename:echo.ext2 --rollup-advance-state=epoch_index:0,input_index_begin:0,input_index_end:1 --server-shutdown -- PATH=/mnt/echo:$PATH /mnt/echo/rollup-init /mnt/echo/echo-dapp  --vouchers=3 --notices=2 --reports=1
```

Inspect request:
```shell
$  cartesi-machine --rollup --server-address=127.0.0.1:10001 --checkin-address=127.0.0.1:10003 --flash-drive=label:hello,filename:echo.ext2 --rollup-inspect-state=query:testquery.bin --server-shutdown -- PATH=/mnt/echo:$PATH /mnt/echo/rollup-init /mnt/echo/echo-dapp --reports=1
```

To generate test input and decode output check `rollup-memory-range.lua` script options from the [emulator](https://github.com/cartesi/machine-emulator) project. 


## Authors

* *Marko Atanasievski*
* *Alex Mikhalevich*

## License

The echo-dapp project and all contributions are licensed under
[APACHE 2.0](https://www.apache.org/licenses/LICENSE-2.0). Please review our [LICENSE](LICENSE) file.
