// Copyright Cartesi and individual authors (see AUTHORS)
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
#[macro_use]
extern crate nix;

use std::fs::File;
use std::os::unix::io::AsRawFd;
use nix::Error;

ioctl_read!(blkgetsize64, 0x12, 114, u64);

pub fn get_blkgetsize64(file: &File) -> Result<u64, Error> {
    let mut size: u64 = 0;
    unsafe {
        blkgetsize64(file.as_raw_fd(), &mut size)?;
    }
    Ok(size)
}

pub mod config;
pub mod dapp_process;
pub mod http_service;
pub mod rollup;
