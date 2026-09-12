///////////////////////////////////////////////////////////////////////////////
//
//  Copyright 2018-2026 Robonomics Network <research@robonomics.network>
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
//
///////////////////////////////////////////////////////////////////////////////
//! Thin binary entry point for the `edge` daemon and toolbox.
//!
//! All logic lives in the library (`edge::cli` and the modules it drives); this
//! wrapper only parses arguments, dispatches, and maps the resulting exit code
//! onto the process status per the CLI contract (0 success, 1 runtime, 2
//! CLI/config, 3 protocol/input).

use std::process::ExitCode;

fn main() -> ExitCode {
    edge::cli::run()
}
