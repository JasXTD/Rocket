# Interface translations

Rocket's interface text is English in the code, wrapped in `tr("…")`, `trf!("…", name)`,
`trn!(count, "… one", "… other")` or, in constant tables, `mark("…")`
(`crates/rocket-ui/src/i18n.rs`). Each language has one folder here: `zh-Hans` for
Simplified Chinese, `ja` for Japanese. Each file maps the English text to its translation:

```json
{
  "Stage all": "全部暂存",
  "Pushed {branch} to {remote}": "已将 {branch} 推送到 {remote}"
}
```

- **One file per part of Rocket**, named for the source it serves: `menus` (menus.rs),
  `chrome` (sidebar, toolbar, status bar, branch switcher), `views` (Changes, Commits,
  Branches, Remotes and the welcome screen), `diff` (the diff view), `preferences`
  (Preferences, General, Appearance). Text more than one part shows goes in `common`.
  A key appears in one file only. A new file is listed in `FILES` in i18n.rs.
- **The key is the English exactly**, including its punctuation and any `…`. Reworded
  English in the code makes the old entry stale; the catalog test names it.
- **Slots stay as they are.** `{branch}`, `{n}` and the like keep their names and may
  move anywhere in the translation. `trn!`'s entry is keyed by its second (plural)
  phrase; Chinese and Japanese need only that one form.
- **Missing is fine while a language is unreleased**: the English shows. A language is
  released by adding it to `RELEASED` in i18n.rs, after which the catalog test requires
  every string.

Check with `cargo test -p rocket-ui --lib i18n::`. To list what is untranslated:
`cargo test -p rocket-ui --lib i18n::tests::untranslated -- --ignored --nocapture`.
To see a language, start Rocket with `ROCKET_LANG=zh-Hans` or `ROCKET_LANG=ja`;
`ROCKET_LANG=pseudo` shows every wrapped string accented and stretched, so plain English
on screen is text that skips `tr`.

## Style

- Keep labels as short as the English or shorter; buttons and the toolbar have fixed room.
- Keep a trailing `…` exactly where the English has one, and add none.
- Keep `·`, `→`, `↑`, `↓`, `\n` and the order of separated parts.
- Keep in Latin letters: Rocket, Git, GitHub, GitLab, Bitbucket, Azure DevOps, Gitea,
  Git LFS, LFS, SSH, OpenSSH, MCP, Lua, AI, URL, HEAD, UTC, Claude, Codex, PowerShell,
  Git Bash, Windows, `.gitignore`, `git log -S`, key names (Ctrl, Shift, Alt, Enter,
  Esc, Tab, F1–F12) and anything in code style.
- Put a half-width space between Chinese or Japanese text and Latin words, numbers or
  slots: `推送到 {remote}`, `{remote} にプッシュ`.
- Chinese uses full-width ，。：；？！（） in sentences; Japanese uses 、。：？！（） and
  「」 for quoted names. English “ ” around a name becomes 「」 in Japanese and “ ” in Chinese.
- ALL-CAPS English captions (BRANCHES, RECENT MESSAGES) are plain text in both languages.
- Sentence fragments that start in lower case continue a line such as "Rebase paused ·";
  translate them as a natural clause. When text names a button (Continue, Skip, Abort,
  Commit), use that button's translation.
- Settings search keywords (long lowercase word lists) become word lists in the
  language; keep the Latin technical words among them (git, ssh, token, lua, mcp).
- Japanese uses です・ます in sentences, and plain nouns or verb stems on buttons and
  menu items (保存, 破棄, 新しいブランチ…). Questions end in ですか？/しますか？.

## Glossary

| English | 简体中文 | 日本語 |
|---|---|---|
| repository | 仓库 | リポジトリ |
| commit (noun, verb) | 提交 | コミット |
| stage / staged | 暂存 / 已暂存 | ステージ / ステージ済み |
| unstage / unstaged | 取消暂存 / 未暂存 | ステージ解除 / 未ステージ |
| stash | 储藏 | スタッシュ |
| fetch | 获取 | フェッチ |
| pull | 拉取 | プル |
| push | 推送 | プッシュ |
| branch | 分支 | ブランチ |
| local branch | 本地分支 | ローカルブランチ |
| remote | 远程 | リモート |
| remote-tracking branch | 远程跟踪分支 | リモート追跡ブランチ |
| upstream | 上游 | アップストリーム |
| ahead / behind | 领先 / 落后 | 先行 / 遅れ |
| tag | 标签 | タグ |
| merge | 合并 | マージ |
| rebase | 变基 | リベース |
| interactive rebase | 交互式变基 | 対話的リベース |
| cherry-pick | 拣选 | チェリーピック |
| revert | 还原 | リバート |
| amend | 修改（上次提交） | 修正（直前のコミット） |
| discard | 放弃 | 破棄 |
| conflict | 冲突 | 競合 |
| resolve / resolution | 解决 / 解决结果 | 解決 / 解決結果 |
| ours / theirs | 我方 / 对方 | 自分側 / 相手側 |
| diff | 差异 | 差分 |
| hunk | 代码块 | ハンク |
| unified / side by side | 统一 / 并排 | 統合 / 左右に並べる |
| working tree | 工作区 | 作業ツリー |
| index | 暂存区 | インデックス |
| worktree (git worktree) | 工作树 | ワークツリー |
| submodule | 子模块 | サブモジュール |
| clone | 克隆 | クローン |
| history | 历史 | 履歴 |
| blame | 追溯 | Blame |
| patch / commit series | 补丁 / 提交系列 | パッチ / コミットシリーズ |
| bisect | 二分查找 | 二分探索 |
| reflog | 引用日志 | reflog |
| detached / unborn | 分离 / 未诞生 | デタッチ / 未作成 |
| signing | 签名 | 署名 |
| identity (name and email) | 身份 | ユーザー情報 |
| hosting | 托管 | ホスティング |
| extension | 扩展 | 拡張機能 |
| custom command | 自定义命令 | カスタムコマンド |
| Preferences | 首选项 | 設定 |
| shortcut | 快捷键 | ショートカット |
| command palette | 命令面板 | コマンドパレット |
| sidebar | 侧边栏 | サイドバー |
| Activity (the drawer) | 活动 | アクティビティ |
| Terminal | 终端 | ターミナル |
| theme / appearance | 主题 / 外观 | テーマ / 外観 |
| Continue / Skip / Abort | 继续 / 跳过 / 中止 | 続行 / スキップ / 中止 |
| File Explorer | 文件资源管理器 | エクスプローラー |
