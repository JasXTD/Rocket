# Interface translations

Rocket's interface text is English in the code, wrapped in `tr("…")`, `trf!("…", name)`,
`trn!(count, "… one", "… other")` or, in constant tables, `mark("…")`
(`crates/rocket-ui/src/i18n.rs`). Each language has one folder here, named by its tag:
`de` German, `fr` French, `sv` Swedish, `ru` Russian, `uk` Ukrainian, `ar` Arabic,
`zh-Hans` Simplified Chinese, `ja` Japanese and `ko` Korean. Swiss German (`de-CH`) has
no folder: Rocket makes it from German, writing ss for ß. Each file maps the English
text to its translation:

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
  move anywhere in the translation.
- **Counted phrases** (`trn!`) are keyed by their second (plural) English phrase. Chinese,
  Japanese and Korean give one text. The other languages give an object with a form per
  CLDR plural category of whole numbers: `one` and `other` for German, Swedish and French
  (French `one` is 0 and 1), `one`, `few` and `many` for Russian and Ukrainian, and
  `zero`, `one`, `two`, `few`, `many` and `other` for Arabic. A form that holds a single
  count (German and Swedish `one`, Arabic `zero`, `one` and `two`) may leave the
  number out:

  ```json
  "{n} files": { "one": "{n} файл", "few": "{n} файла", "many": "{n} файлов" }
  ```
- **Missing is fine for a preview translation**: the English shows. All built-in
  catalogs are selectable in Preferences → General → Languages; unfinished choices
  carry a Preview label. A language becomes complete by joining `RELEASED` in
  i18n.rs, after which the catalog test requires every string. The dropdown initially
  shows the first supported system language (including previews), or English. An
  explicit choice overrides detection and survives restart.

Check with `cargo test -p rocket-ui --lib i18n::`. To list what is untranslated:
`cargo test -p rocket-ui --lib i18n::tests::untranslated -- --ignored --nocapture`.
Choose a language in Preferences for normal use. For development, start Rocket with
`ROCKET_LANG=<tag>` (de, de-CH, fr, sv, ru, uk, ar, zh-Hans, ja or ko) to override the
system default; an explicit saved preference still takes precedence.
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
- Korean uses 합니다/하세요 in sentences, and short nouns or verb stems on buttons.
- French puts a no-break space (U+00A0) before `:` `;` `?` `!` and uses « » around
  quoted names; German writes ß where the spelling needs it (Swiss German is made from it).
- Ukrainian is translated from the English, not from the Russian.
- Arabic is written in ordinary logical order with Arabic punctuation (، ؟ ؛) and no
  direction marks: Rocket's patched text layout (`vendor/epaint/patch.md`) draws it right
  to left with Latin words and numbers in their own order. The interface itself is not
  mirrored.

## Glossary: Chinese and Japanese

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

## Glossaries of the other languages

Each translator's terms, to keep later work consistent with them.

### Deutsch (de; Swiss German follows it)

Formal address (Sie) in messages, infinitives on commands and buttons. Write ß; the Swiss variant swaps it for ss.

| English | Deutsch |
|---|---|
| repository | Repository (Plural: Repositorys) |
| commit (noun, verb) | Commit (der, Plural: Commits) / committen, committet |
| stage / staged | stagen / gestagt |
| unstage / unstaged | unstagen / nicht gestagt |
| stash | Stash (der, Plural: Stashes) / stashen |
| fetch | Fetch / fetchen |
| pull | Pull / pullen |
| push | Push / pushen |
| branch | Branch (der, Plural: Branches) |
| local branch | lokaler Branch |
| remote | Remote (das, Plural: Remotes) |
| remote branch | Remote-Branch |
| remote-tracking branch | Remote-Tracking-Branch |
| upstream | Upstream (der) |
| ahead / behind | voraus / zurück |
| merge | Merge (der) / mergen, gemergt |
| merge commit | Merge-Commit |
| rebase | Rebase (der) / rebasen |
| interactive rebase | interaktiver Rebase |
| cherry-pick | Cherry-Pick (der) / cherry-picken |
| revert | Revert (der) / reverten |
| amend | ändern (Commit ändern, Ändern fortsetzen) |
| discard | verwerfen |
| conflict | Konflikt |
| resolve / resolution | lösen / Lösung |
| ours / theirs / base | unsere / deren / Basis |
| diff | Diff (der) |
| hunk | Hunk |
| unified / side by side / split | Inline / Nebeneinander / Geteilt |
| working tree | Arbeitsverzeichnis |
| working file | Arbeitsdatei |
| index | Index |
| worktree (git worktree) | Worktree (der, Plural: Worktrees) |
| submodule | Submodul (Plural: Submodule) |
| tag | Tag (das, Plural: Tags) |
| clone | klonen |
| checkout / check out | Checkout / auschecken, ausgecheckt |
| history | Verlauf |
| path history | Pfadverlauf |
| blame | Blame |
| patch / commit series | Patch / Commit-Serie |
| bisect | Bisect |
| reflog | Reflog |
| detached / unborn | losgelöst / ungeboren |
| signing | Signierung / signieren |
| identity | Identität (Git-Identität) |
| hosting | Hosting (Hosting-Dienst, Hosting-Konto) |
| extension | Erweiterung |
| custom command | eigener Befehl (Eigene Befehle) |
| Preferences / settings | Einstellungen |
| shortcut | Tastenkombination |
| command palette | Befehlspalette |
| sidebar | Seitenleiste |
| drawer (Activity, Terminal) | Bereich (Aktivitätsbereich, Terminalbereich) |
| Activity | Aktivität |
| Terminal | Terminal |
| theme / appearance | Design / Darstellung |
| Continue / Skip / Abort | Fortsetzen / Überspringen / Abbrechen |
| Stop | Stoppen |
| review (Rocket's check before acting) | prüfen / Prüfung |
| preview (read-only copy) | Vorschau |
| Summary (repository page) / summary (commit) | Übersicht / Zusammenfassung |
| Messages (composer menu) | Vorlagen |
| map | Karte |
| AI | KI |
| File Explorer | Datei-Explorer |

### Svenska (sv)

Address the user as du; imperatives on commands and buttons. Common English Git words with Swedish inflection (committa, pusha, merga, rebasa) where the native word is long or unusual; Git's own Swedish (köa, gren, arbetskatalog) where it is short and established.

| English | Svenska |
|---|---|
| repository | repo (ett repo, repot, repon) |
| commit (noun, verb) | commit (en commit, commiten, commits) / committa, committad |
| stage / staged | köa / köad, köade |
| unstage / unstaged | ta ur kön / oköad, oköade |
| stash | stash (en stash, stashen, stashar) / stasha |
| fetch | hämta / hämtning |
| pull | pull / pulla |
| push | push / pusha |
| branch | gren (grenen, grenar) |
| local branch | lokal gren |
| remote | remote (en remote, remoten, remotes) |
| remote branch | fjärrgren |
| remote-tracking branch | fjärrspårande gren |
| upstream | upstream (upstream-grenen) |
| ahead / behind | före / efter |
| merge | merge (en merge, mergen) / merga, mergad |
| merge commit | merge-commit |
| rebase | rebase (en rebase, rebasen) / rebasa |
| interactive rebase | interaktiv rebase |
| cherry-pick | cherry-pick / cherry-picka |
| revert | revert / reverta |
| amend | ändra (Ändra commit, Fortsätt ändringen) |
| discard | kasta |
| conflict | konflikt |
| resolve / resolution | lösa / lösning |
| ours / theirs / base | vår / deras / bas |
| diff | diff (en diff, diffen) |
| hunk | stycke |
| unified / side by side / split | Inline / Sida vid sida / Delad |
| working tree | arbetskatalog |
| working file | arbetsfil |
| index | index |
| worktree (git worktree) | worktree (pl. worktrees) |
| submodule | undermodul |
| tag | tagg (pl. taggar) |
| clone | klona / kloning |
| checkout / check out | utcheckning / checka ut, utcheckad |
| history | historik |
| path history | sökvägshistorik |
| blame | blame |
| patch / commit series | patch (pl. patchar) / commit-serie |
| bisect | bisect |
| reflog | reflog |
| detached / unborn | frånkopplad / ofödd |
| signing | signering |
| identity | identitet (Git-identitet) |
| hosting | hosting / värdtjänst |
| extension | tillägg |
| custom command | anpassat kommando |
| Preferences / settings | Inställningar |
| shortcut | kortkommando |
| command palette | kommandopalett |
| sidebar | sidofält |
| drawer (Activity, Terminal) | panel (aktivitetspanelen, terminalpanelen) |
| Activity | Aktivitet |
| Terminal | Terminal |
| theme / appearance | tema / utseende |
| Continue / Skip / Abort | Fortsätt / Hoppa över / Avbryt |
| Stop | Stoppa |
| review (Rocket's check before acting) | granska / granskning |
| preview (read-only copy) | förhandsvisning |
| Summary (repository page) / summary (commit) | Översikt / sammanfattning |
| Messages (composer menu) | Mallar |
| map | karta |
| AI | AI |
| File Explorer | Utforskaren |
| File menu | Fil (not Arkiv: the same key labels file inputs) |

### Français (fr)

| English | Français |
|---|---|
| repository | dépôt |
| commit (noun) | commit (m.) ; as a lone label or button: Commit |
| commit (verb) | valider (Valider et pousser, Valider 2 fichiers indexés) |
| stage / staged | indexer / indexé |
| unstage / unstaged | désindexer / non indexé |
| stash (noun / verb) | remise (f.) / remiser |
| fetch | récupérer / récupération |
| pull | tirer |
| push | pousser / poussée (URL de poussée) |
| branch | branche |
| local branch | branche locale |
| remote | dépôt distant (adj. distant) |
| remote-tracking branch | branche de suivi |
| upstream | amont (branche amont) |
| ahead / behind | en avance / en retard |
| merge | fusionner / fusion |
| fast-forward | avance rapide |
| rebase | rebaser / rebasage |
| interactive rebase | rebasage interactif |
| cherry-pick | cherry-pick (m.), kept in English |
| revert | inverser / inversion |
| amend | amender / amendement |
| discard | rejeter |
| abort | abandonner |
| conflict | conflit |
| resolve / resolution | résoudre / résolution |
| ours / theirs | notre version / leur version (Garder la nôtre / la leur) |
| diff | diff (m.) |
| hunk | section |
| unified / side by side / split | unifié / côte à côte / scindé |
| working tree | copie de travail |
| index | index |
| worktree (git worktree) | arbre de travail |
| submodule | sous-module |
| tag | étiquette |
| clone | cloner / clonage |
| checkout / checked out | extraire / extraite |
| switch (branch) | basculer |
| history | historique |
| blame | blame |
| patch / commit series | patch (pl. patchs) / série de commits |
| bisect | bissection |
| reflog | reflog |
| recovery | restauration ; reprise for paused operations |
| detached / unborn | détachée / sans commit |
| signing | signature |
| identity | identité |
| hosting | hébergement |
| issue | ticket |
| extension | extension |
| custom command | commande personnalisée |
| Preferences / Settings | Préférences / Paramètres |
| shortcut | raccourci |
| command palette | palette de commandes |
| sidebar | barre latérale |
| drawer | panneau |
| Activity | Activité |
| Terminal | Terminal |
| theme / appearance | thème / apparence |
| refresh / reload | actualiser / recharger |
| collapse / expand | replier / déplier |
| preview | aperçu |
| Continue / Skip / Abort | Continuer / Passer / Abandonner |
| File Explorer | Explorateur de fichiers |

### Русский (ru)

| English | Русский |
|---|---|
| repository | репозиторий |
| commit (noun / verb) | коммит / закоммитить; the Commit button and field label: «Коммит», «Commit…» menu item: «Закоммитить…» |
| stage / staged | индексировать, добавить в индекс / в индексе; tight buttons: «В индекс», «Всё в индекс» |
| unstage / unstaged | убрать из индекса / не в индексе; tight buttons: «Из индекса», «Всё из индекса» |
| stash (verb / noun) | спрятать / спрятанное, спрятанные изменения (as VS Code) |
| fetch | получить (получение) |
| pull | вытянуть |
| push | отправить (отправка) |
| branch | ветка |
| local branch | локальная ветка |
| remote | удалённый репозиторий |
| remote-tracking branch | удалённая ветка |
| upstream | вышестоящая ветка (as VS Code and git) |
| ahead / behind | впереди / позади |
| merge | слияние / слить |
| rebase | перебазирование / перебазировать |
| interactive rebase | интерактивное перебазирование |
| cherry-pick | перенос / перенести (коммит) |
| revert | откат / откатить (коммит) |
| amend | исправить (последний коммит) |
| discard | отменить (изменения) |
| undo / redo | отменить / повторить |
| conflict | конфликт |
| resolve / resolution | разрешить / разрешение |
| ours / theirs | наша версия / их версия; short labels «Наша», «Их» |
| diff | различия |
| hunk | фрагмент |
| unified / side by side | единый / рядом |
| working tree | рабочее дерево |
| index | индекс |
| worktree (git worktree) | рабочее дерево (plural «Рабочие деревья») |
| check out / checked out | извлечь / извлечена |
| submodule | подмодуль |
| tag | тег |
| clone | клонировать |
| history | история |
| blame | авторство (строк) |
| patch / commit series | патч / серия коммитов |
| bisect | двоичный поиск |
| reflog | reflog |
| detached / unborn | отделён / без коммитов |
| signing | подпись |
| identity (name and email) | данные автора |
| hosting | хостинг |
| extension | расширение |
| custom command | пользовательская команда |
| Preferences / settings | Настройки |
| shortcut | сочетание клавиш |
| command palette | палитра команд |
| sidebar | боковая панель |
| Activity (the drawer) | Операции |
| Terminal | Терминал |
| theme / appearance | тема / оформление |
| preview (temporary clone) | предпросмотр |
| Continue / Skip / Abort | Продолжить / Пропустить / Прервать |
| File Explorer | Проводник |

### Українська (uk)

| English | Українська |
|---|---|
| repository | репозиторій |
| commit (noun / verb) | коміт / закомітити; the Commit button and field label: «Коміт», «Commit…» menu item: «Закомітити…» |
| stage / staged | індексувати, додати до індексу / в індексі; tight buttons: «До індексу», «Усе до індексу» |
| unstage / unstaged | прибрати з індексу / поза індексом; tight buttons: «З індексу», «Усе з індексу» |
| stash (verb / noun) | сховати / схованка (plural «Схованки») |
| fetch | отримати (отримання) |
| pull | витягнути |
| push | надіслати (надсилання) |
| branch | гілка |
| local branch | локальна гілка |
| remote | віддалений репозиторій |
| remote-tracking branch | віддалена гілка |
| upstream | відстежувана гілка |
| ahead / behind | попереду / позаду |
| merge | злиття / злити |
| rebase | перебазування / перебазувати |
| interactive rebase | інтерактивне перебазування |
| cherry-pick | перенесення / перенести (коміт) |
| revert | відкат / відкотити (коміт) |
| amend | виправити (останній коміт) |
| discard | відкинути (зміни) |
| undo / redo | скасувати / повторити |
| conflict | конфлікт |
| resolve / resolution | розв'язати / розв'язання |
| ours / theirs | наша версія / їхня версія; short labels «Наша», «Їхня» |
| diff | відмінності |
| hunk | фрагмент |
| unified / side by side | об'єднаний / поруч |
| working tree | робоче дерево |
| index | індекс |
| worktree (git worktree) | робоче дерево (plural «Робочі дерева») |
| check out / checked out | перейти, перемкнутися / активна |
| submodule | підмодуль |
| tag | тег |
| clone | клонувати |
| history | історія |
| blame | авторство (рядків) |
| patch / commit series | патч / серія комітів |
| bisect | двійковий пошук |
| reflog | reflog |
| detached / unborn | від'єднано / без комітів |
| signing | підпис |
| identity (name and email) | дані автора |
| hosting | хостинг |
| extension | розширення |
| custom command | власна команда |
| Preferences / settings | Налаштування |
| shortcut | сполучення клавіш |
| command palette | палітра команд |
| sidebar | бічна панель |
| Activity (the drawer) | Операції |
| Terminal | Термінал |
| theme / appearance | тема / вигляд |
| preview (temporary clone) | перегляд |
| Continue / Skip / Abort | Продовжити / Пропустити / Перервати |
| File Explorer | Провідник |

### 한국어 (ko)

| English | 한국어 |
|---|---|
| repository | 리포지토리 |
| commit (noun, verb) | 커밋 |
| stage / staged | 스테이지 / 스테이지됨 (스테이지된 파일) |
| unstage / unstaged | 스테이지 취소 / 스테이지 안 됨 |
| stash | 스태시 |
| fetch | 페치 |
| pull | 풀 |
| push | 푸시 |
| branch | 브랜치 |
| local branch | 로컬 브랜치 |
| remote | 원격 |
| remote-tracking branch | 원격 추적 브랜치 |
| upstream | 업스트림 |
| ahead / behind | 앞섬 / 뒤처짐 |
| merge | 병합 |
| rebase | 리베이스 |
| interactive rebase | 대화형 리베이스 |
| cherry-pick | 체리픽 |
| revert | 되돌리기 |
| amend | 수정 (마지막 커밋 수정) |
| discard | 폐기 |
| conflict | 충돌 |
| resolve / resolution | 해결 / 해결 결과 |
| ours / theirs | 내 쪽 / 상대 쪽 |
| base (merge base) | 베이스 |
| diff | diff |
| hunk | 헝크 |
| unified / side by side / split | 통합 / 나란히 / 분할 |
| working tree | 작업 트리 |
| working file | 작업 파일 |
| index | 인덱스 |
| worktree (git worktree) | 워크트리 |
| submodule | 서브모듈 |
| tag | 태그 |
| clone | 복제 |
| checkout / check out | 체크아웃 |
| history | 기록 |
| blame | Blame |
| patch / commit series | 패치 / 커밋 시리즈 |
| import / export | 가져오기 / 내보내기 |
| bisect | 이진 탐색 |
| reflog | reflog |
| detached / unborn | 분리됨 / 커밋 없음 |
| signing | 서명 |
| identity (name and email) | 사용자 정보 |
| hosting | 호스팅 |
| extension | 확장 |
| custom command | 사용자 지정 명령 |
| dependency | 의존성 |
| Preferences | 환경 설정 |
| Settings | 설정 |
| shortcut | 단축키 |
| command palette | 명령 팔레트 |
| sidebar | 사이드바 |
| drawer / pane | 패널 |
| Activity (the drawer) | 활동 |
| Terminal | 터미널 |
| theme / appearance | 테마 / 모양 |
| preview | 미리 보기 |
| draft | 초안 |
| review (check before acting) / AI review | 검토 / AI 리뷰 |
| operation | 작업 |
| refresh / reload / loading | 새로 고침 / 다시 불러오기 / 불러오는 중 |
| Continue / Skip / Abort | 계속 / 건너뛰기 / 중단 |
| Stop / paused | 중지 / 일시 중지됨 |
| File Explorer | 파일 탐색기 |

Conventions:

- Messages use 합니다/하세요; questions end in 하시겠습니까?. Buttons and menu items are noun phrases (저장, 브랜치 삭제, 새 브랜치).
- After a slot, Korean particles depend on the unknown last sound, so a noun follows the slot where it reads naturally ({branch} 브랜치를, {remote} 원격을, {path} 파일을). Otherwise the text uses particles that never change (에, 에서, 의), or 을(를) as Korean software does.
- Counts are noun + {n}개 (파일 {n}개, 커밋 {n}개). Korean has no plural forms, so each counted phrase is one text.
- Views and buttons named inside sentences are in single quotes: '변경 사항', '계속', '커밋 건너뛰기'.
- A parenthesis follows the word with no space: 닫기(Esc), 요약(필수).

### العربية (ar)

| English | العربية |
|---|---|
| repository | مستودع (ج. مستودعات) |
| commit (noun / verb) | إيداع (ج. إيداعات) / أودِع؛ زر: إيداع |
| stage / staged | تجهيز (جهِّز) / مُجهَّز |
| unstage / unstaged | إلغاء التجهيز (ألغِ تجهيز) / غير مُجهَّز |
| stash (noun / verb) | مخبأ (ج. مخابئ) / تخبئة (خبِّئ) |
| fetch | جلب (اجلب) |
| pull | سحب (اسحب) |
| pull request | طلب سحب |
| push | دفع (ادفع) |
| branch | فرع (ج. فروع) |
| local branch | فرع محلي |
| remote | مستودع بعيد (ج. مستودعات بعيدة) |
| remote-tracking branch | فرع تتبُّع بعيد (ج. فروع تتبُّع بعيدة) |
| upstream | المنبع؛ tracks → يتتبّع |
| ahead / behind | متقدم / متأخر |
| checkout / checked out | استخراج / مُستخرَج |
| merge | دمج |
| fast-forward | التقديم السريع (fast-forward) |
| rebase | إعادة التأسيس |
| interactive rebase | إعادة التأسيس التفاعلية |
| cherry-pick | انتقاء |
| revert | عكس |
| amend | تعديل (آخر إيداع) |
| discard | إهمال (أهمِل) |
| ignore (.gitignore) | تجاهل؛ ignored files → الملفات المتجاهلة |
| conflict | تعارض (ج. تعارضات) |
| resolve / resolution | حلّ / حل (ج. حلول) |
| ours / theirs | نسختنا / نسختهم |
| base | الأساس |
| diff | الفروق |
| hunk | كتلة (ج. كتل) |
| unified / side by side / split | موحّد / جنبًا إلى جنب / مقسّم |
| working tree | شجرة العمل |
| index | الفهرس |
| worktree (git worktree) | شجرة عمل (ج. أشجار العمل) |
| submodule | وحدة فرعية (ج. وحدات فرعية) |
| tag | وسم (ج. وسوم) |
| clone | استنساخ |
| history | السجل |
| blame | نسب الأسطر |
| patch / commit series | رقعة (ج. رقع) / سلسلة إيداعات |
| bisect | البحث الثنائي |
| reflog | سجل المراجع |
| detached / unborn | منفصل / بلا إيداعات |
| parent / children (commits) | الأب (ج. الآباء) / الأبناء |
| signing | التوقيع |
| identity (name and email) | الهوية |
| hosting | الاستضافة |
| extension | إضافة (ج. إضافات) |
| custom command | أمر مخصص (ج. أوامر مخصصة) |
| Preferences / Settings | التفضيلات / الإعدادات |
| shortcut | اختصار (ج. اختصارات) |
| command palette | لوحة الأوامر |
| sidebar | الشريط الجانبي |
| Activity (the drawer) | النشاط |
| Terminal | الطرفية |
| drawer / pane | الدرج / الجزء |
| shell | الصدفة |
| workspace | مساحة العمل |
| theme / appearance | السمة / المظهر |
| Continue / Skip / Abort | متابعة / تخطي / إلغاء |
| File Explorer | مستكشف الملفات |
| Actions (CI page) | Actions (يبقى بالحروف اللاتينية) |
