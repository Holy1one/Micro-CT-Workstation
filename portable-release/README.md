# portable-release/：本地免安装包输出

本目录存放 `npm.cmd run portable:build` 生成的免安装交付包：

- `micro-ct-workstation-portable.exe`：唯一免安装交付入口；
- `build-info.json`：构建时间、源提交、工作区是否含未提交改动和 EXE 的 SHA256。

两者都是可再生成的编译/打包产物，不进入版本控制：根 `.gitignore` 忽略 `portable-release/*`，只保留本 README。本文件是目录中唯一被跟踪的内容，用来在干净检出中保留该目录，并使 `module-map` 的 `release-artifacts` 归属继续成立。

发布约定：

1. 在 Windows 上运行 `npm.cmd run portable:build`，构建会写入上面的 EXE 与 `build-info.json`；
2. 双击 EXE 验证启动后，把它作为本机构建产物分发，不要提交到 Git；
3. 需要版本留档时给对应源码提交打 tag（例如 `v0.7.2`），由接收方按 tag 重新构建，而不是从仓库下载二进制；
4. `target/release/` 只是 Cargo/Tauri 编译输出与缓存，同样不提交。
