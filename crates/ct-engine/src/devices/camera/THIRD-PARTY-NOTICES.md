# Native camera protocol sources

## Necessary Nikon protocol adaptation

The operation/property/event constants, packed rational shutter representation,
Nikon event dataset layout, focus-drive direction parameters, and D7100 Live View
header offsets used in `nikon.rs`, `ptp.rs` and `live_view.rs` were adapted from:

- Project: [digiCamControl](https://github.com/dukus/digiCamControl)
- Files: `CameraControl.Devices/Nikon/NikonBase.cs`, `NikonD600Base.cs`,
  `NikonD7100.cs`, and `CameraControl.Devices/BaseMTPCamera.cs`
- Local reference: `Related_files/digiCamControl-master/CameraControl.Devices/`
- Copyright (C) 2014 Duka Istvan
- License: MIT; the source files and repository `LICENSE` carry this notice.

Only the necessary Nikon protocol elements are adapted. The application does
not execute digiCamControl or link its framework. PortableDeviceLib code and
closed vendor DLLs are excluded.

### MIT permission notice

digiCamControl - DSLR camera remote control open source software
Copyright (C) 2014 Duka Istvan

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Independent WPD implementation and dependencies

`wpd.rs` independently implements Microsoft's documented WPD interfaces, not a
translation of PortableDeviceLib. API references:

- [Supporting MTP extensions](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/supporting-mtp-extensions)
- [Read-data command](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/wpd-command-mtp-ext-execute-command-with-data-to-read)
- [End-data-transfer response](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/wpd-command-mtp-ext-end-data-transfer)
- [Windows Rust IPortableDevice](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Devices/PortableDevices/struct.IPortableDevice.html)

The `windows` bindings are MIT OR Apache-2.0; `image` is MIT OR Apache-2.0;
`getrandom` is MIT OR Apache-2.0. Exact dependency versions and transitive
dependencies are fixed by `Cargo.lock`. Distribution tooling must include their
applicable third-party licenses with the offline candidate package.
