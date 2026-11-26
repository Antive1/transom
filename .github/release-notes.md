**transom** – a full-width top bar (AppBar) for Windows 11, written in Rust. Early preview (alpha).

| File | What |
|---|---|
| `transom-setup-<version>-x64.exe` | **Installer** – per-user, no admin rights, optional start with Windows |
| `transom-<version>-x64-portable.zip` | Portable – unzip and run `transom-app.exe` |
| `SHA256SUMS.txt` | Checksums |

> [!WARNING]
> **Windows may block the first start.** The installer is not code signed yet, so SmartScreen shows a blue
> *"Windows protected your PC"* window. Click **More info → Run anyway**. You can verify the download against
> `SHA256SUMS.txt` (`Get-FileHash transom-setup-<version>-x64.exe`).

Requires Windows 11 x64. See the [README](https://github.com/Antive1/transom#readme) for features and known limitations.

Built from this tag by GitHub Actions. [Code signing policy](https://github.com/Antive1/transom/blob/main/CODE_SIGNING_POLICY.md):
free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).
