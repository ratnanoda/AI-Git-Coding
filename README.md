# Rust yt-dlp GUI

Rust製のシンプルなWindows向けyt-dlp GUIです。

## 機能

- URLを貼り付けてダウンロード
- 動画 MP4 / 音声 M4Aを選択
- 保存先フォルダを選択
- ダウンロードログ表示
- 実行中のダウンロード停止
- 初回起動時に公式GitHub Releaseから `yt-dlp.exe` を自動取得

## Windows版EXEの取得方法

1. GitHubの **Actions** を開く
2. **Build Windows EXE** を開く
3. 最新の成功した実行を開く
4. Artifactsにある **Rust-yt-dlp-GUI-Windows** をダウンロード
5. ZIPを展開し、`Rust-yt-dlp-GUI.exe` を起動

Windows DefenderのSmartScreenが表示された場合は、配布元不明の未署名EXEであるためです。ソースコードとActionsのビルド内容を確認してから実行してください。

## ローカルビルド

```powershell
cargo build --release
```

生成先:

```text
target\release\rust-ytdlp-gui.exe
```

## 注意

ダウンロードするコンテンツについて、利用規約・著作権・地域の法律を守って使用してください。
