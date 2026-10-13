# Konsep UI baru cobserve

*Laporan riset untuk keputusan owner, 2026-10-13. Sumber: lima catatan riset di `docs/research/research_notes/Konsep UI baru cobserve/` ditambah layar cobserve hari ini (`docs/screenshots/*.txt`, `DESIGN.md`, `GUIDE.md`, `CLAUDE.md`). Kalimat yang ditandai **(inferensi)** adalah penalaran peneliti atau penulis, bukan fakta bersumber. Istilah teknis, nama tool, crate, key, dan SQL dibiarkan dalam bahasa Inggris.*

**Simulasi browser** dari konsep ini — HOME, grammar resource → inspector, palette `:`, floating session, popover `e`, setup wizard, dengan keymap yang jalan — ada di https://claude.ai/artifact/WkRxfkk2VHU2narocc4pE9 (sumbernya `docs/research/simulasi-konsep-ui.html`; ukuran 80×24, 120×36, 160×48, tema terang/gelap).

**Rekomendasi utama: jangan tulis ulang cobserve dari nol — tulis ulang *shell UI*-nya dari nol di atas mesin yang sudah ada.** Dari ~44 ribu baris Rust, yang menentukan "konsep UI" hanyalah `src/ui/*` dan state navigasi di `src/app.rs` (sekitar 15 ribu baris); matematika §5, sumber data, history, insight, tape, PTY, prayer, dan generator `FAKE=1` tidak bergantung pada konsep layar mana pun dan justru menjadi aset yang membuat redesign ini murah dan aman **(inferensi dari layout `src/`)**. Konsep baru yang diusulkan: satu layar **HOME "apa yang salah sekarang"** sebagai pintu masuk, satu **grammar resource → inspector** ala k9s/lazygit untuk semua objek (node, query, orang, job, DAG, tiket, sesi, event), satu **command palette `:`** yang menjadi sumber tunggal bagi footer, `?`, dan pencarian global, sesi PTY sebagai **floating pane** di atas monitor, **setup wizard** dengan Keychain, dan popover **"explain this number"** yang mengubah aturan "setiap angka terlacak ke rumus §5" menjadi fitur yang bisa dipakai. Visualisasi baru semuanya bertekstur — stacked share bar yang persis berjumlah 100 %, bullet bar dengan tick ambang, braille/area history, heatmap node×menit, Gantt Airflow, kalender jam kerja, density strip tape — sehingga warna tetap hanya berarti severity dan semuanya bertahan di 16 warna dan mono. Fitur baru diurutkan dari yang "diam-diam tak tergantikan": history SQLite dengan scrubber membuka sepertiga daftar lainnya, fingerprint query dari `system.query_log`, health row (parts/merges/replicas/disk), catatan + ekspor laporan insiden, snooze, OSC 8 link; semua tetap read-only ke ClickHouse, dan tiga-empat kandidat yang menulis ke sistem lain ditandai eksplisit. Stack tetap ratatui 0.30.2 + crossterm 0.29; ekosistem sudah hampir seluruhnya di 0.30, kecuali beberapa crate basi yang disebut untuk dihindari. Urutan pembangunan: fake data dulu, shell baru dibangun paralel di balik flag sampai paritas, lalu shell lama dihapus — sama seperti disiplin §11 hari ini.

## Masalahnya bukan kecantikan, melainkan jumlah pintu

cobserve hari ini sudah memiliki tiga primitif discoverability yang dimiliki tool terbaik — overlay `?`, footer kontekstual, `⏎`/`esc` yang konsisten — tetapi membelanjakannya dengan buruk. Overlay `?` pada 120×36 adalah satu dinding ~22 baris kunci plus 10 baris legenda untuk *semua* view sekaligus, dan beberapa barisnya terpotong `…` ([120x36-help.txt](../../screenshots/120x36-help.txt)). Footer NODES membawa **delapan hint** dalam ~110 kolom (`↑↓ move · ⏎ open · c SQL on it · ←→ fold · tab insights · u pivot · / filter · s sort · ␣ healthy`) ([00-context.md](../research_notes/Konsep%20UI%20baru%20cobserve/00-context.md)). Masthead memuat **delapan tab pada digit** dengan `0` berarti "mesin ini" dan `7` berarti "sampai lima puluh sesi", sementara sesi sendiri hidup di balik prefix `ctrl+\` yang tidak diiklankan apa pun di layar sampai pengguna sudah berada di dalam sesi ([GUIDE.md §6, §8.1](../../../GUIDE.md)). Konfigurasi hanya lewat environment variable ([CLAUDE.md](../../../CLAUDE.md)). Empat baris chrome (masthead, day line, FLEET, REDASH) ditambah footer menyisakan 31 dari 36 baris pada ukuran target dan 19 dari 24 pada ukuran minimum **(inferensi)**.

Ini bukan masalah unik cobserve. Survei Julia Evans atas ~1.100 jawaban menempatkan "remembering syntax" (115), "keyboard shortcut inconsistencies" (84), dan "poor discoverability" (55) di antara frustrasi terminal terbesar ([jvns.ca](https://jvns.ca/blog/2025/02/05/some-terminal-frustrations/)). Empat tool yang tidak saling berhubungan dan dipakai luas — k9s, lazygit, Helix, yazi — secara independen menetap pada vocabulary yang sama: `?` yang *terbatas pada konteks fokus*, `/` untuk filter lokal, `esc` selalu mundur satu tingkat, dan `:` atau palette untuk segala hal lain ([k9s README](https://github.com/derailed/k9s#readme), [lazygit Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md), [Helix keymap](https://docs.helix-editor.com/keymap.html), [yazi quick start](https://yazi-rs.github.io/docs/quick-start)). Textualize menyatakan tujuan palette-nya secara eksplisit: "once you learn how to use one Textual app, you can use them all" dan "users only need to type two or three characters to get to any command" ([Textual 0.37 blog](https://textual.textualize.io/blog/2023/09/15/textual-0370-adds-a-command-palette/)). Mengikuti konvensi itu adalah kemenangan usability dengan sendirinya: pengguna k9s atau lazygit sudah tahu cara memakai cobserve baru sebelum membukanya **(inferensi)**.

Dari sisi *chrome*, konvensi 2024–2026 adalah border membawa *makna* (fokus, modalitas), bukan pengelompokan: Helix mengirim popup tanpa border secara default (`popup-border: none`), zellij memakai `pane_frame_style: titles` — "single line above panes" — sebagai default, dan Textual memformalkan tangga `background → surface → panel → boost` sebagai pengganti kotak ([Helix editor config](https://docs.helix-editor.com/editor.html), [zellij options](https://zellij.dev/documentation/options.html), [Textual design guide](https://textual.textualize.io/guide/design/)). Kotak penuh di mana-mana (btop) kini terbaca sebagai "classic monitor", bukan modern **(inferensi peneliti)**. Kabar baiknya: gaya cobserve hari ini — title-rule `─ clickhouse3 · pay-ch-node-1 ─────` tanpa kotak, fokus lewat gutter `▌` — sudah berada di sisi yang benar dari konvensi itu, dan aturan "warna = severity saja" justru *mendukung* chrome tanpa border karena border berwarna akan bersaing dengan merah/amber/hijau data **(inferensi peneliti)**.

## Konsep: satu HOME, satu grammar resource, satu palette

Konsep yang diusulkan meminjam empat pola yang terbukti dan menggabungkannya di bawah aturan cobserve. Dari **k9s**: resource list dengan crumbs bar, `esc` mundur, `:` sebagai pintu utama (`:pod`, `:pulses` "monitoring dashboard showing cluster health") ([k9s README](https://github.com/derailed/k9s)). Dari **lazygit**: panel bertumpuk di kiri plus satu main panel, `+`/`_` memutar screen mode normal → half → full, `mainPanelSplitMode: flexible` ("split horizontally if wide enough, otherwise vertically"), jump key di judul panel ([lazygit Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)). Dari **btop/bottom**: `e` membesarkan satu widget sampai memenuhi layar ([bottom README](https://github.com/ClementTsang/bottom)). Dari **zellij**: floating pane dan status bar which-key yang berubah per mode ([zellij README](https://github.com/zellij-org/zellij)). Dari **posting/Textual**: footer yang "always shows the most useful keys for whatever is focused" dan palette `Ctrl+P` yang "lists everything Posting can do" ([posting guide](https://posting.sh/guide/)).

Struktur layarnya tetap empat tingkat, tetapi satu baris lebih hemat dari hari ini: **masthead** (nama, profil, tab bernama, live dot, jam), **day line** (tetap — garis waktu shalat adalah identitas, bukan dekorasi), **crumbs line** yang merangkap sub-tab dan hint `: command / filter`, **band ringkas satu baris** (FLEET │ REDASH), body, **footer** maksimal lima hint. Kartu band yang lebih besar pindah ke HOME dan hanya kembali ke setiap layar pada ≥ 160 kolom. Digit dibatasi pada enam hal, mengikuti kebiasaan 1–5 lazygit ([lazygit README](https://github.com/jesseduffield/lazygit)): `0 home`, `1 fleet` (nodes · map · heat · tape sebagai sub-view yang diputar `tab`), `2 queue`, `3 airflow`, `4 jira`, `5 sessions`; LOCAL menjadi kartu di HOME dan `:local`. Di bawah 100 kolom digit menghilang dari masthead dan namanya tetap **(inferensi; usulan peneliti usability)**.

### HOME — "apa yang salah sekarang" (120×36)

Daftar INSIGHTS hari ini sudah mengurutkan masalah dari yang terburuk; HOME mempromosikannya menjadi layar pertama lintas *semua* sumber, dengan `⏎` membawa ke objeknya dan crumbs memberi jalan pulang. Density strip TAPE dan heatmap fleet di bawahnya menjawab "kapan mulai buruk" sebelum satu baris pun dibaca.

```
 ❖ cobserve · prod     0 home   1 fleet   2 queue   3 airflow   4 jira   5 sessions                ● live   15:52:07 WIB
 ━━ Subuh 04:21 ━━━━ Terbit 05:33 ━━━━ Dzuhur 11:45 ━━━━ Ashar 14:48 ━●┄┄ Maghrib 17:50 · in 1h58m ┄┄┄┄ Isya 18:59 ┄┄
 home                                                                                        : command   / find   ? keys

 ─ FLEET · 9 nodes · 1 hot ─── ─ REDASH · 16 waiting ─────── ─ AIRFLOW · last 24 h ─────── ─ JIRA · mine ───────────────
 mem ━━━╸━━━┊━┊ 36.4% 280/768G oldest 3m43s ✖ · 3 stale      217 runs · ✔ 211 · ✖ 3        5 open · 1 overdue ✖
 cpu ━━━╺━━━┊━┊ 28.7% 55/192 c ●●●●●●○ 6/7 workers busy      ▸ 2 running · ◌ 1 · ↻ 1       today 5h10 of 7h30 logged
 15 queries · 4 !  lag ≤12s    ▇████▇▆██ waiting · 4 min     ✖ 1 stuck for 262 d           due this week 3

 ─ WHAT'S WRONG NOW · 8 · worst first ─────────────────────────────────────────────────────────────────────────────────
▌✖ FLEET    clickhouse3     memory 93.1% 59.6/64 GiB · CPU 93.4% 14.9/16 · the server itself holds 47.4%      ⏎ node
 ✖ QUEUE    queries         16 waiting · oldest 3m43s · 4/4 workers hold a runaway (ch3 ch-bi ch7)              ⏎ queue
 ✖ AIRFLOW  test_clickhouse_connection   running 262d04h · stuck since Jan 15 · 0/1 tasks                       ⏎ run
 ▲ FLEET    clickhouse-bi   memory 67.2% 43.0/64 GiB ↗ full in ~18m · j.petrova holds 33.1%                     ⏎ node
 ▲ QUEUE    j.petrova       #8585 AML dashboard · 5m38s · 21.2 GiB on clickhouse-bi · 95% done                  ⏎ job
 ▲ AIRFLOW  cbk_accounts_report          failed 07:15 at upload_report · 3rd failure this week                   ⏎ run
 ▲ JIRA     DATA-8821       due today · 0h logged · in review                                                   ⏎ ticket
 ● LOCAL    this Mac        Claude plan · week 71% 142/200 h · battery 34%                                       ⏎ local

 ─ LAST 60 MIN · events per minute ───────────────── 15:00 ──────── 15:15 ──────── 15:30 ──────── 15:45 ──── now ─
 ✖ ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▂▃▇█▇▂▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁   8 ✖ · burst 15:26–15:29 · m.kairys runaway ×4
 ▲ ▁▁▁▁▂▁▁▁▁▁▁▁▂▁▁▁▁▁▁▁▂▄▆▆▆▄▂▁▁▁▁▂▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁  18 ▲

 ─ FLEET · max memory per minute · ░ <40%  ▒ <70%  ▓ <90%  █ ≥90% ─────────────────────────────────────────────────────
 clickhouse3     ▒▒▒▒▒▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓███████████████████████████████████████  93.1%  59.6/64 GiB
 clickhouse-bi   ░░░▒▒▒▒▒▒▒▒▓▓▓▓▓▓▒▒▒▒▒▒▒▓▓▓▓▓▓▓▓▓▒▒▒▒▒▒▒▒▒▒▒▓▓▓▓▓▓▓▓▒▒▒▒▒▒▒▒  67.2%  43.0/64 GiB
 clickhouse7     ░░░░░░░░▒▒▒▒▒▒▒▒░░░░░░░░▒▒▒▒▒▒▒▒░░░░░░░░▒▒▒▒▒▒▒▒░░░░░░░░▒▒▒▒  59.9%  38.3/64 GiB
 clickhouse2     ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▒▒▒▒▒░░░░░  41.8%  26.8/64 GiB
 5 nodes ≤ 38%   ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  all ≤ 38%    ␣ unfold

 ─ clickhouse3 · pay-ch-node-1:9000 ───────────────────────────────────────────────────────────── e explain · ⏎ open ─
 MEM 59.6/64 GiB 93.1%  ━━━━━━━━━━━━━━━━━━━━━━━┊━━━┊╸   CPU 14.9/16 cores 93.4%  ━━━━━━━━━━━━━━━━━━━━━━━━┊━━━┊╸
 queries 45.7% + server 47.4% · 4 running (2 !) · free 4.4 GiB = 0.2× a 24.0 GiB query · ▂▃▄▅▆▇▇█ → steady
 ✖ memory 93.1% · ✖ CPU 93.4% · ▲ grigol.gankava 8m31s 16.3 GiB · ▲ lag 0 s
                                                                                                    ✓ link copied ▕
 ↑↓ move   ⏎ go there   e explain   / find   : command   ? keys                           9/9 nodes · 0.8 s · read-only
```

Yang berubah dari INSIGHTS hari ini: kolom sumber (`FLEET QUEUE AIRFLOW JIRA LOCAL`) sehingga satu daftar melayani lima view, glyph runaway menjadi `!` agar tidak lagi hampir identik dengan `✖` critical (peneliti usability menandai `✖` vs `✕` sebagai dua bentuk serupa untuk dua makna) **(inferensi)**, dan toast `✓ link copied` muncul sebagai tumpukan kanan-bawah, bukan di footer. Band ringkas tidak hilang: di HOME ia justru mekar menjadi kartu, di layar lain menyusut ke satu baris.

### FLEET — resource list + inspector (120×36)

Pohon node → user → query tetap, tetapi kolom MEM menjadi **stacked share bar**: segmen setiap user duduk pada offset tempatnya hidup di bar induk, sehingga invarian §5.3 (Σ user + server = node) *terlihat* tanpa membaca angka. Identitas segmen dibawa tekstur `█ ▓ ▒ ░ ·`, bukan warna; severity (merah ≥ 90 %) berlaku pada seluruh bar node **(inferensi peneliti visualisasi)**. Sparkline 4 menit tetap di baris node. Inspector di 120×36 adalah drawer bawah; tombol `f` membuatnya memenuhi body (zoom btop) dan crumbs bertambah satu tingkat.

```
 ❖ cobserve · prod     0 home   1 fleet   2 queue   3 airflow   4 jira   5 sessions                ● live   15:52:07 WIB
 ━━ Subuh 04:21 ━━━━ Terbit 05:33 ━━━━ Dzuhur 11:45 ━━━━ Ashar 14:48 ━●┄┄ Maghrib 17:50 · in 1h58m ┄┄┄┄ Isya 18:59 ┄┄
 home › fleet › nodes › clickhouse3                  nodes · map · heat · tape ‹tab›              : command   / filter
 FLEET mem ━━━╸━━━━┊━┊ 36.4% 280/768 GiB   cpu ━━━╺━━━━┊━┊ 28.7% 55/192 cores   15 q · 4 !  │  REDASH 16 wait · 3m43s ✖
 NODE · USER → PERSON           SHARE OF NODE · mem             MEM ▼   GiB      CPU ▼  cores    Q     LONGEST  4 MIN
▌▾ clickhouse3                  ████████▓▓▓▓▒▒░░░░░░░░░░░░░░··  93.1%  59.6/64  93.4%  14.9/16  4 !2  8m31s !  ▂▃▄▅▆▇▇█
 ├ r_redash → grigol.gankava    ████████                        25.5%  16.3     19.4%  3.10     1     8m31s !
 ├ r_redash → j.petrova                 ▓▓▓▓                    12.9%  8.23      4.4%  0.70     1     6m40s !
 ├ airflow                                  ▒▒                   5.5%  3.53     31.2%  5.00     1       18s
 └ server · caches · merges                   ░░░░░░░░░░░░░░    47.4%  30.3     36.5%  5.84
 ▾ clickhouse-bi                ██████████▓▓▓▓▓▒▒▒░░··········  67.2%  43.0/64  71.7%  11.5/16  3 !1  6m40s !  ▆▆▃▅▆▆▄▅
 ├ r_redash → j.petrova         ██████████                      33.1%  21.2     11.9%  1.90     1     6m40s !
 ├ r_redash → m.kairys                    ▓▓▓▓▓                 14.1%  9.00     18.1%  2.90     1     2m12s !
 ├ airflow                                     ▒▒▒               9.4%  6.00     21.3%  3.40     1       41s
 └ server · caches · merges                       ░░             10.6%  6.80     20.4%  3.30
 ▸ 5 healthy folded · ch4 ch5 ch6 ch8 ch9 · all ≤ 38%                                                        ␣ unfold

 ─ INSIGHTS · 2 · tab to go to one ────────────────────────────────────────────────────────────────────────────────────
 ✖ clickhouse3     memory 93.1% · CPU 93.4% · the server itself holds 47.4%                                    +1 more
 ▲ clickhouse-bi   memory 67.2% ↗ full in ~18m · j.petrova holds 33.1%                                         +1 more
 ▲ clickhouse7     r.simonyte 5m02s · 51.3 GiB · wallet.ledger · lag 12 s

 ─ clickhouse3 · pay-ch-node-1:9000 ─────────────────────────────────────────────── e explain · f zoom · c SQL on it ─
 MEM 59.6/64 GiB 93.1%  ━━━━━━━━━━━━━━━━━━━━━━━┊━━━┊╸   CPU 14.9/16 cores 93.4%  ━━━━━━━━━━━━━━━━━━━━━━━━┊━━━┊╸
                        ▲ 4 min ago 52.1 GiB      70% 90%                            ▲ 4 min ago 13.0 cores  70% 90%
  64 ┤                                              ▗▄▟██████████  mem 59.6 GiB · hi 59.6
  32 ┤ ⢀⣀⣀⣀⠤⠤⠒⠒⠉⠉⠑⠒⠤⣀⣀⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣀⠤⠒⠉⠉⠉⠉⠉⠉⠉⠉⠉⠉  cpu 14.9 cores · hi 15.1
     │▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄
   0 ┼───────────────────────────────────────────────────────────  -4m ──────── -2m ──────── now   [ ] window

 ▎ 8f1c · r_redash → grigol.gankava · #7438 Gateway transfers · 8m31s · 16.3 GiB · 3.1 cores · usually 42 s (318× / 7 d)
 ▎ SELECT toStartOfHour(ts) AS h, countIf(state = 'failed') AS failed
 ▎ FROM open_banking.ais WHERE ts >= now() - INTERVAL 6 HOUR GROUP BY h
 ▎ ORDER BY h                                              E estimate · would read 1.9 B rows · 412 parts · 23 k marks

 ↑↓ move   ⏎ drill   ⌫ back   / filter   : command                                        9/9 nodes · 0.8 s · read-only
```

Bar MEM/CPU di inspector adalah **bullet bar** Stephen Few: track tebal, tick ambang `┊` pada 70 % dan 90 % (ambang amber/merah yang sudah dipakai insight dan tape hari ini), dan penanda pembanding `▲ 4 min ago` — Few mendesain bullet graph tepat "to replace the meters and gauges that are often used on dashboards" karena desainnya yang linear muat di ruang kecil ([Bullet Graph Design Spec](https://www.perceptualedge.com/articles/misc/Bullet_Graph_Design_Spec.pdf), [FusionCharts primer](https://www.fusioncharts.com/resources/chart-primers/bullet-graph)). Baris `usually 42 s (318× / 7 d)` adalah fitur E1 (fingerprint dari `system.query_log`, lihat bagian fitur). Chart braille/area memakai `GraphType::Area` + `Dataset::fill_to_y()` yang masuk di ratatui 0.30.1 dan rendering "Braille over Blocks" dari 0.30.0 untuk menumpuk garis CPU di atas area memori ([ratatui CHANGELOG](https://github.com/ratatui/ratatui/blob/main/CHANGELOG.md)).

### 160×48 — inspector menjadi kolom kanan, panel samping bertumpuk

Pada ≥ 150 kolom, `mainPanelSplitMode: flexible` ala lazygit mengubah drawer menjadi kolom kanan 46 kolom, dan ringkasan empat sumber menjadi panel bertumpuk di kiri dengan jump key `[1]…[4]` di judul (lazygit `showPanelJumps`) ([lazygit Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)). Baris node memuat *kedua* bar (share + CPU bullet) dan sparkline 20 sel; heatmap menampilkan 60 menit penuh. Sketsa 16 baris pertama:

```
 ❖ cobserve · prod     0 home   1 fleet   2 queue   3 airflow   4 jira   5 sessions                                                  ● live   15:52:07 WIB  
 ━━━ Subuh 04:21 ━━━━━━━━ Terbit 05:33 ━━━━━━━━ Dzuhur 11:45 ━━━━━━━━ Ashar 14:48 ━━━━━●┄┄┄┄ Maghrib 17:50 · in 1h58m ┄┄┄┄┄┄┄┄ Isya 18:59 ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄      
 home › fleet › nodes › clickhouse3                         nodes · map · heat · tape ‹tab›                                             : command   / filter    
─[1] fleet ─────────── NODE · USER → PERSON     SHARE OF NODE · mem      MEM ▼  GiB     CPU ━━┊━┊   %      cores    ─ clickhouse3 · pay-ch-node-1 ──────────────
9 nodes · 1 hot        ▌▾ clickhouse3           ██████▓▓▓▒░░░░░░░░░░░░·· 93.1%  59.6/64 ━━━━━━━┊━┊╸ 93.4%  14.9/16  MEM 59.6/64 GiB 93.1%  ━━━━━━━━━━━━━┊━━┊╸   
mem 36.4% 280/768 GiB   ├ r_redash → grigol.g…  ██████                   25.5%  16.3    ━╸━━━━━┊━┊  19.4%  3.10     CPU 14.9/16 c   93.4%  ━━━━━━━━━━━━━┊━━┊╸   
cpu 28.7% 55/192 c      ├ r_redash → j.petrova        ▓▓▓                12.9%  8.23    ╸━━━━━━┊━┊   4.4%  0.70     free 4.4 GiB = 0.2× a 24 GiB query          
─[2] redash ──────────  ├ airflow                        ▒                5.5%  3.53    ━━━╸━━━┊━┊  31.2%  5.00      64 ┤                    ▗▄▟██████████      
16 waiting · 3m43s ✖    └ server · caches · m…            ░░░░░░░░░░░░   47.4%  30.3    ━━━━╸━━┊━┊  36.5%  5.84      32 ┤ ⢀⣀⣀⠤⠒⠉⠑⠒⠤⣀⡀⠀⠀⠀⠀⠀⢀⣀⠤⠒⠉⠉⠉⠉⠉⠉⠉⠉          
6/7 busy · 3 stale      ▾ clickhouse-bi         ████████▓▓▓▒▒░░········· 67.2%  43.0/64 ━━━━━━━┊╸┊  71.7%  11.5/16      │▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄   
─[3] airflow ─────────  ├ r_redash → j.petrova  ████████                 33.1%  21.2    ━╸━━━━━┊━┊  11.9%  1.90       0 ┼──────────────────────────────  -4m now
217 runs · ✖ 3 · ▸ 2    ├ r_redash → m.kairys           ▓▓▓              14.1%  9.00    ━━╸━━━━┊━┊  18.1%  2.90     ─ insights ─────────────────────────────────
✖ 1 stuck 262 d         ├ airflow                          ▒▒             9.4%  6.00    ━━╸━━━━┊━┊  21.3%  3.40     ✖ memory 93.1% 59.6/64 · ✖ CPU 93.4% 14.9/16
─[4] jira ────────────  └ server · caches · m…               ░░          10.6%  6.80    ━━╸━━━━┊━┊  20.4%  3.30     ▲ grigol.gankava 8m31s · usually 42 s (318×)
5 open · 1 overdue ✖    ▸ 5 healthy folded · ch4 ch5 ch6 ch8 ch9 · all ≤ 38%                               ␣ unfold ─ SQL · 8f1c · grigol.gankava ──────────────
today 5h10 of 7h30                                                                                                  SELECT toStartOfHour(ts) AS h, countIf(…    
```

### 80×24 — selalu "zoom", denominator pindah ke header

Terminal kecil tidak mendapat layout lain; ia hanya *selalu* berada dalam mode zoom pada satu region. Band menyusut ke satu baris, sparkline hilang, denominator naik ke header kolom (`MEM % of GiB`) sehingga aturan "setiap persentase mencetak denominatornya" tetap dipenuhi dengan denominator yang *terlihat di layar* meski tidak di setiap sel **(inferensi peneliti usability)**. `⏎` menukar daftar dengan inspector penuh (push ke crumbs), `esc` kembali. Inilah cara §5.3 tetap jujur di 80 kolom: kolom yang tidak muat dibuang seluruhnya, bukan dipotong.

```
 ❖ cobserve   0 home 1 fleet 2 queue 3 airflow 4 jira 5 sess   ● 15:52 WIB
 ━ Subuh 04:21 ━━ Dzuhur 11:45 ━━ Ashar 14:48 ●┄ Maghrib 17:50 in 1h58m ┄ Isya
 home › fleet › nodes                                       : cmd   / filter
 FLEET 36.4% 280/768 GiB · 28.7% 55/192 c · 4 !  │ REDASH 16 wait · 3m43s ✖
 NODE · USER → PERSON        MEM % of GiB     CPU % of cores   Q    LONGEST
▌▾ clickhouse3               93.1%  59.6/64   93.4%  14.9/16   4 !2  8m31s !
 ├ r_redash → grigol.gankava 25.5%  16.3      19.4%  3.10      1     8m31s !
 ├ r_redash → j.petrova      12.9%  8.23       4.4%  0.70      1     6m40s !
 ├ airflow                    5.5%  3.53      31.2%  5.00      1       18s
 └ server · caches · merges  47.4%  30.3      36.5%  5.84
 ▸ clickhouse-bi             67.2%  43.0/64   71.7%  11.5/16   3 !1  6m40s !
 ▸ 5 healthy folded · ch4 ch5 ch6 ch8 ch9 · ≤ 38%                   ␣ unfold

 ─ INSIGHTS · 2 ─────────────────────────────────────────────────────────────
 ✖ clickhouse3    memory 93.1% · CPU 93.4% · server holds 47.4%      +1 more
 ▲ clickhouse-bi  memory 67.2% ↗ full in ~18m · j.petrova 33.1%     +1 more

 ⏎ opens the inspector full-screen · esc comes back





 ↑↓ ⏎ / : ?                                       9/9 · 0.8 s · read-only
```

### Sesi — floating pane di atas monitor, dan sebagai resource list

Hari ini sesi adalah view `7` yang menggantikan monitor. Konsep baru memberi dua jalan. `5 sessions` adalah **resource list** biasa (nama, jenis, folder, server, state `● working ○ idle ! approval ✕ stopped`, umur, baris terakhir) dengan `/` dan `⏎` membuka penuh — grammar yang sama dengan node dan DAG. Dan `ctrl+\` membuka **floating pane** 80 %×70 % di atas view mana pun, pola floating pane zellij ([zellij README](https://github.com/zellij-org/zellij)), sehingga menonton clickhouse3 sambil menulis SQL atau bertanya ke Claude tidak lagi berarti berpindah tab. Bar yang muncul setelah prefix hari ini *sudah* berperilaku seperti which-key; ia hanya perlu *terlihat* seperti itu (dua kolom `cand → desc`, tata letak `[which]` yazi) dan muncul 250 ms setelah prefix (Helix `idle-timeout`) agar pengguna cepat tidak pernah melihatnya ([yazi theme](https://yazi-rs.github.io/docs/configuration/theme), [Helix editor config](https://docs.helix-editor.com/editor.html)). Widget PTY-nya adalah milik cobserve sendiri (`portable-pty 0.9` + `vt100 0.16`, sudah di `Cargo.toml`); `tui-term 0.3.4` memakai pasangan yang sama persis sehingga tidak ada alasan menggantinya ([crates.io via catatan design-language](https://crates.io/crates/tui-term)).

```
 home › fleet › nodes › clickhouse3                  nodes · map · heat · tape ‹tab›       sessions ‹ctrl+\›   : cmd
 FLEET mem ━━━╸━━━━┊━┊ 36.4% 280/768 GiB   cpu ━━━╺━━━━┊━┊ 28.7% 55/192 cores   15 q · 4 !  │  REDASH 16 wait · 3m43s ✖
 NODE · USER → PERSON           SHARE OF NODE · mem             MEM ▼   GiB      CPU ▼  cores    Q     LONGEST  4 MIN
▌▾ clickhouse3    ╭─ 7 claude · ~/code/cobserve · ● working ──────────────────── 8 ○ opencode · 9 ▪ sql ch3 · + new ─╮
 ├ r_redash → gr  │ › why is clickhouse3 at 93% memory right now?                                                    │
 ├ r_redash → j.  │                                                                                                  │
 ├ airflow        │ The server itself holds 47.4% (30.3 of 64 GiB): caches and merges, not queries. Two Redash       │
 └ server · cach  │ queries hold another 38.4%. The one to look at is grigol.gankava's #7438 — 8m31s, 16.3 GiB,      │
 ▾ clickhouse-bi  │ against a 7-day median of 42 s for the same fingerprint …                                        │
 ├ r_redash → j.  │                                                                                                  │
 ├ r_redash → m.  │ ──────────────────────────────────────────────────────────────────────────────────────────────── │
 ├ airflow        │ ❯ █                                                                                              │
 └ server · cach  ╰─ ctrl+\ hide · ctrl+\ 1-5 a view · ctrl+\ 7-9 ↑↓ a session · ctrl+\ f full · ctrl+\ / find ──────╯
 ▸ 5 healthy folded · ch4 ch5 ch6 ch8 ch9 · all ≤ 38%                                                        ␣ unfold
```

Di dalam pane, setiap key milik program (seperti hari ini); agar `ctrl+\`, `⇧⏎`, `ctrl+⏎` dapat diandalkan, app mengaktifkan `PushKeyboardEnhancementFlags(DISAMBIGUATE_ESCAPE_CODES | REPORT_ALTERNATE_KEYS)` saat `supports_keyboard_enhancement()` menjawab ya — dipanggil *sebelum* event loop karena fungsi itu "will block and possibly time out while `crossterm::event::read` or `crossterm::event::poll` are being called" ([crossterm docs](https://docs.rs/crossterm/latest/crossterm/terminal/fn.supports_keyboard_enhancement.html)). Protokol keyboard kitty diimplementasikan Ghostty, iTerm2, WezTerm, Alacritty, foot, Rio, Warp, Windows Terminal, dan **xterm.js**, sehingga desktop shell dapat menjanjikannya tanpa syarat ([kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)). Di terminal tanpa dukungan, `F1–F10` tetap menjadi fallback dan satu toast mengatakannya.

### Command palette `:` — satu tabel aksi, tiga renderer

Palette adalah lapisan yang hilang dari discoverability cobserve, dan ia *skalabel* melewati ~15 perintah tempat footer dan `?` berhenti. Rancangannya mengikuti Textual: fuzzy ("match the keys you type in the same order, but not necessarily at the start of the command"), karakter yang cocok disorot, `discover()` mengisi daftar saat input kosong ([Textual command palette](https://textual.textualize.io/guide/command_palette/)). Yang membuatnya murah: **satu `Vec<Action { id, label, keys, scope, enabled, run }>`** adalah sumber tunggal untuk footer (lima aksi teratas yang enabled pada scope fokus), `?` (semua aksi scope itu, dapat difilter dengan mengetik), dan palette (semua aksi semua scope + channel pencarian objek) — satu komponen untuk dibangun, satu untuk dipelajari **(inferensi peneliti usability)**. Saat kosong, `discover()` menampilkan "apa yang salah sekarang" dulu, lalu aksi untuk baris di bawah kursor. Channel (`tab`): actions · nodes · people · queries (SQL dan id) · jobs · DAGs · tickets · sessions · tape — model "channels" television ([television](https://docs.rs/television)). Pengaturan jarang (`:theme light`, `:clock utc`, `:prayer off`, `:setup redash`, `:density compact`) hidup di sini, bukan di footer.

```
                  ╭─ : ─────────────────────────────────────────────────────────────────────────╮
                  │ › exp█                                                                      │
                  ├─────────────────────────────────────────────────────────────────────────────┤
                  │▌ EXPlain this number           the 93.1% under the cursor · its §5 formula   e│
                  │  EXPlain estimate · this query  EXPLAIN ESTIMATE → rows · parts · marks       E│
                  │  EXPort incident report         markdown → clipboard and ~/incidents/         y│
                  │  EXPand all nodes               unfold every node in the tree                 →│
                  │  setup · airflow (EXPired?)     the login was refused 4m58s ago · re-enter     │
                  ├─────────────────────────────────────────────────────────────────────────────┤
                  │ ↑↓ choose   ⏎ run   tab channel › actions · nodes · people · queries · jobs … │
                  ╰─────────────────────────────────────────────────────────────────────────────╯▒
                   ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒
```

(`EXP` kapital hanya menandai karakter yang disorot bold di layar nyata.) Bayangan datang dari `Block::shadow()` 0.30, popup dari `frame.area().centered(...)` + `Clear` ([Block docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Block.html), [center recipe](https://ratatui.rs/recipes/layout/center-a-widget/)). Implementasi: `tui-input 0.15.5` untuk baris input, `nucleo` (matcher Helix, dipakai television) untuk skor, ~300 baris sendiri; `ratatui-command-palette 0.1.0` dari ratatui-labs layak *dipelajari* (lima renderer: Modal, FlatOverlay, SplitPreview, Fullscreen, InlineDropdown) tetapi berlabel experimental dan berisiko dijadikan dependency ([docs.rs/ratatui-command-palette](https://docs.rs/ratatui-command-palette)).

### Setup wizard — gh sebagai cetak biru, Keychain sebagai brankas

Konfigurasi *bisa* dimasukkan dan disimpan dari dalam app dengan aman dengan meniru bentuk `gh auth login`: satu pertanyaan per layar dengan satu baris "mengapa", token "securely in the system credential store" dengan fallback plaintext yang *diumumkan* ("Authentication credentials saved in plain text") ([gh auth login manual](https://cli.github.com/manual/gh_auth_login), [cli/cli login_flow.go](https://github.com/cli/cli/blob/trunk/pkg/cmd/auth/shared/login_flow.go)). Crate `keyring` memberi macOS Keychain, Secret Service, dan Windows Credential Manager lewat satu API, "so secrets never touch application-controlled disk files in plaintext" ([docs.rs/keyring](https://docs.rs/keyring/)). Non-rahasia ke `~/.config/cobserve/config.toml` (XDG, seperti k9s dan lazygit), nilai string boleh merujuk env var `${CH_PASSWORD}` seperti profil Harlequin ([harlequin README](https://github.com/tconbeer/harlequin)), dan env var tetap menjadi lapisan override teratas sesuai clig ("Flags > Environment variables > … configuration"; "Never *require* a prompt"; `--no-input`) ([clig.dev](https://clig.dev/)) — jadi `CH_SEED_URLS=…` hari ini tetap berjalan dan §9 "environment-only" menjadi "environment-first". Urutan langkah menurut payoff: ClickHouse → layar FLEET pertama; Redash → QUEUE menyala; Airflow; Jira; kota shalat. Setiap langkah berakhir dengan **pemeriksaan hidup** di poller yang sudah ada, bukan dengan "saved". Satu blob JSON di satu entri Keychain menghindari lima dialog izin macOS **(inferensi peneliti usability)**. Layar tanpa konfigurasi menawarkan `d` — fleet yang di-generate (`FAKE=1` sudah ada), agar pendatang baru melihat UI sebelum mengetik satu URL pun.

```
   ╭─ setup · 1 of 5 · ClickHouse ──────────────────────────────────────────────────────────────╮
   │                                                                                            │
   │  Where is ClickHouse?                                                                      │
   │  One or more HTTP addresses, comma-separated. cobserve reads system.clusters from them     │
   │  to discover the rest of the fleet. Nothing is ever written to ClickHouse.                 │
   │                                                                                            │
   │  seeds     › http://pay-ch-node-1:8123█                                                    │
   │  cluster   › ch_paysera                                                                    │
   │  user      › cobserve_ro                                                                   │
   │  password  › ••••••••••••           → macOS Keychain · service cobserve · account ch:prod   │
   │  timeout   › 5000 ms                over the VPN a poll takes 0.6–5 s; 1500 marks nodes ↯  │
   │                                                                                            │
   │  checking…  9 nodes answered in 0.8 s · pay-ch-node-3 refused the password                 │
   │                                                                                            │
   │  ⏎ next: Redash    tab next field    esc skip this step    d try it with a generated fleet │
   ╰────────────────────────────────────────────────────────────────────────────────────────────╯
```

### "Explain this number" — aturan rumah menjadi fitur paling khas

Tidak satu pun TUI yang diriset memiliki affordance "jelaskan nilai ini"; cobserve unik mampu menawarkannya karena aturannya sendiri mewajibkan setiap angka terlacak ke rumus §5 **(inferensi peneliti usability)**. `e` (atau hover ≥ 700 ms dengan mouse) pada sel angka membuka popover: rumus, input beserta nilainya saat ini, tabel sumber, waktu poll, dan rujukan §5 — sekaligus alat debugging paritas dengan FleetLens. Implementasinya: setiap metrik membawa `Provenance { formula_id, inputs: Vec<(name, value, unit, source)>, at }`; popover hanya me-render-nya. Biayanya adalah menyalurkan provenance lewat struct stat, yang secara konseptual sudah dituntut aturan §5.

```
▌▾ clickhouse3                  ████████▓▓▓▓▒▒░░░░░░░░░░░░░░··  93.1%  59.6/64  93.4%  14.9/16  4 !2  8m31s !  ▂▃▄▅▆▇▇█
                                 ╭─ explain · 93.1% ──────────────────────────────────────────────────────────╮
                                 │ node.mem_pct = node.mem_used ÷ node.mem_total × 100                        │
                                 │              = 59.6 GiB ÷ 64.0 GiB × 100 = 93.1 %                          │
                                 │                                                                            │
                                 │ mem_used    59.6 GiB   §6.1 fallback chain      system.asynchronous_metrics │
                                 │ mem_total   64.0 GiB   OSMemoryTotal            system.asynchronous_metrics │
                                 │ polled 2 s ago · pay-ch-node-1:8123 · answered in 0.6 s                    │
                                 │                                                                            │
                                 │ DESIGN.md §5.3 · FleetLens fleetMetrics.ts                   y copy · esc │
                                 ╰────────────────────────────────────────────────────────────────────────────╯
 ├ r_redash → j.petrova         ██████████                      33.1%  21.2     11.9%  1.90     1     6m40s !
```

(Nama input di popover ini ilustratif; nama sebenarnya mengikuti rantai fallback §6.1 pada node itu.) Untuk angka turunan — `cores(q)` dari §5.2 — popover menunjukkan kedua bentuk (delta dan rata-rata) dan mana yang sedang berlaku, sehingga keterbatasan yang didokumentasikan CLAUDE.md ("a just-started query can briefly claim far more CPU than the node is using") terbaca di layar, bukan di README **(inferensi)**.

## Katalog visualisasi: tekstur membawa identitas, warna membawa severity

Semua bentuk di bawah mengikuti satu checklist yang disusun peneliti visualisasi dari Tufte, Few, dan praktik btop: denominator tetap dan tercetak (`Sparkline::max()`, bukan auto-scale — "auto-scaled sparklines in a fleet grid lie"), terbaru di kanan dengan panjang jendela sebagai satu label, fold per slice memakai *maksimum* bukan rata-rata ("did it spike?"), identitas lewat glyph/tekstur dan severity lewat warna — tidak pernah sebaliknya, ambang sebagai tick pada bar, hapus border yang bukan data, satu skala untuk semua small multiples, heatmap 5 tingkat, raster hanya untuk artefak ([Tufte, Sparkline theory and practice](https://www.edwardtufte.com/notebook/sparkline-theory-and-practice-edward-tufte/), [Bullet Graph Design Spec](https://www.perceptualedge.com/articles/misc/Bullet_Graph_Design_Spec.pdf), [btop_draw.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_draw.cpp), [ratatui Sparkline](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Sparkline.html)). Ramp warna tetap **berundak, bukan kontinu**: cream (< 70 %), amber (70–90 %), merah (≥ 90 %) — gradien 101 langkah btop indah tetapi membuat "agak oranye" tidak berarti apa pun yang presisi, dan setiap warna di cobserve harus terlacak ke ambang §5 **(inferensi peneliti visualisasi)**.

**Stacked share bar (Σ = 100 %).** ratatui tidak punya widget stacked bar; ~60 baris: lebar segmen dihitung dalam seperdelapan sel (`round(share × W × 8)`) dengan **largest-remainder rounding** agar segmen berjumlah tepat `W×8`, karena satu sel kosong nyasar akan secara kasat mata melanggar invarian §5.3. Identitas: `█` user A, `▓` user B, `▒` user C, `░` server · caches · merges, `·` free; lebih dari empat user dilipat ke "others" seperti pohon melipat node sehat. Pie chart ditolak: `tui-piechart 1.0.3` ada, tetapi pie di grid sel 2:1 mendistorsi sudut dan butuh hue untuk irisan ([tui-piechart](https://docs.rs/tui-piechart)). dust adalah preseden terdekat (bar blok dengan "shades of grey … to indicate which parent folder a subfolder belongs to") ([dust README](https://github.com/bootandy/dust)).

```
 clickhouse3   mem 59.6/64 GiB  93.1%
 ████████████████▓▓▓▓▓▓▓▓▒▒▒▒░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░····
 █ grigol.gankava 25.5% 16.3   ▓ j.petrova 12.9% 8.23   ▒ airflow 5.5% 3.53   ░ server 47.4% 30.3   · free 6.9% 4.4
```

**Braille/area history.** Teknik btop: dua sampel per kolom dipadatkan ke satu glyph braille/blok, diskalakan ke `max_value` tetap, "Braille over Blocks" sejak 0.30.0 memungkinkan area memori (level) di blok dan garis CPU (rate) di braille ([btop_draw.cpp](https://github.com/aristocratos/btop/blob/main/src/btop_draw.cpp), [ratatui CHANGELOG](https://github.com/ratatui/ratatui/blob/main/CHANGELOG.md)). Tiga label sumbu saja (0, setengah, penuh) karena header sudah mencetak denominator; margin kanan membawa nilai kini + puncak jendela (konvensi titik Tufte dalam bentuk angka). Jendela `4m → 1h → 24h` dengan `[`/`]`; data per jendela harus sudah ada (ring buffer 1 s / 15 s / 6 min, atau SQLite — fitur A1), tidak pernah diinterpolasi. Untuk seri panjang, downsample dengan min/max per bucket atau LTTB — "naive downsampling … can miss sharp spikes and troughs" ([oneuptime on LTTB](https://oneuptime.com/blog/post/2026-03-31-clickhouse-largesttrianglebuckets/markdown), [Chart.js decimation](https://chartjs.org/docs/next/configuration/decimation.html)); ClickHouse punya `largestTriangleThreeBuckets(N)` jika history 24 jam pernah dibaca dari `system.metric_log`. Mock-up ada di inspector FLEET di atas.

**Heatmap node × menit.** Bentuk paling padat dan jujur untuk "seluruh fleet, satu jam terakhir": satu glyph per (node, menit) dari lima tingkat ` ░▒▓█` (berfungsi di mono), warna severity hanya pada sel di atas ambang. 40 node × 60 menit = 40×60 sel, muat di 160×48; di 120×36 tampilkan 30 menit atau lipat baris sehat. Lima tingkat, bukan delapan: `▁–█` dibaca sebagai *tinggi* dan duduk di baseline sehingga baris terlihat seperti bar chart; GitHub, git-cal, dan lnav semua berhenti di 4–5 tingkat ([git-cal](https://github.com/CurbaiCode/git-cal), [lnav UI docs](https://docs.lnav.org/en/stable/ui.html)). Mock-up ada di HOME. `malevich 1.24.1` menyediakan heatmap untuk ratatui 0.30 dan patut dievaluasi; `trazo 0.0.1` menjanjikan heatmap/stacked/area tetapi berumur dua bulan — item pantau, bukan dependency ([awesome-ratatui](https://github.com/ratatui/awesome-ratatui), [trazo](https://docs.rs/trazo)).

**Bullet bar.** Track `━━━╸━━━` hari ini ditambah tick ambang `┊` dan penanda pembanding `▲ 4 min ago` / "this time yesterday" (butuh A1). Bands kualitatif Few sebagai shade latar tidak bisa dilakukan pada satu baris tanpa warna, maka dua tick menggantikannya; pada kartu dua baris, baris `░▒▓` di bawah bar. Varian ringkas untuk baris tabel: tick saja, warna berubah tepat saat melewati tick. Tidak ada dial: gauge busur braille butuh ≥ 7×4 sel, mengkodekan satu angka, dan tidak satu pun tool yang disurvei (btop, bottom, k9s, sampler) menggambar dial ([sampler README](https://raw.githubusercontent.com/sqshq/sampler/master/README.md)).

```
 mem  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╸━━━━━━━━━━┊━━━━━┊━━  59.6/64 GiB  93.1%
                                     ▲ 4 min ago 52.1          70%   90%
 93.1%  ━━━━━━━━━━━━━┊━━┊╸  59.6/64      ← red: past the 90 % tick
 25.5%  ━━━╸━━━━━━━━━┊━━┊━  16.3         ← cream: under both
```

**Gantt Airflow per task.** Baris ACTIVITY hari ini (`▪·✖▸◌━`, satu glyph per slot) sudah merupakan Gantt terminal di tingkat DAG; perbaikannya adalah Gantt sejati untuk DAG *terpilih*: satu baris per task, `━` untuk rentang berjalan diletakkan menurut menit mulai/selesai, `▸` kini, `✔` selesai, `✖` gagal, `◌` queued, `↻` retry, grid jam yang sama. 24 jam dalam 96 kolom = 15 menit per sel; task yang lebih pendek dari satu sel tetap mendapat `▪`. flowrs, satu-satunya TUI Airflow yang ditemukan, tidak menggambar timeline — ini adalah diferensiasi nyata ([flowrs](https://github.com/jvanbuel/flowrs)); desain ini diturunkan dari Grid/Gantt web UI Airflow, bukan dari preseden terminal **(inferensi peneliti visualisasi)**.

```
 statement_daily_agg_reload · 13:39 → running 2h12m · 4/7 tasks · usually 1h48m (median of 30)
 task                   13          14          15          16
 extract_statements     ━━━━✔
 build_partitions           ━━━━━━━✔
 validate_counts                   ━━✔
 reload_partitions                   ━━━━━━━━━━━━━━▸              running 1h05m · ▲ 2.1× usual
 refresh_views                                     ◌              queued
 notify                                            ◌
 ping_clickhouse        ✖↻ ✖↻ ✖                                   3 tries · failed
```

**Kalender jam kerja.** View "Time logged" Jira hari ini sudah menggambar bar chart harian yang mencetak jamnya; tambahkan heatmap bulanan gaya GitHub (7 baris hari × 5 kolom minggu, 5 tingkat terhadap norma 7h30) sebagai bentuk ringkas yang memperlihatkan ritme mingguan; widget `Calendar` ratatui (feature `widget-calendar`) dapat menampungnya langsung lewat style per tanggal ([ratatui widgets](https://docs.rs/ratatui/latest/ratatui/widgets/index.html)).

```
 October 2026 · 120h in 16 working days      ░ <2h  ▒ <5h  ▓ <7h30  █ ≥7h30
        W40  W41  W42  W43  W44
 Mon         █    █    ▓    █
 Tue         ▓    █    █    ▒
 Wed         ·    ▓    █
 Thu    █    ▒    █    █
 Fri    █    ▓    ░    ▓
```

**Density strip TAPE.** HIST view lnav ("a stacked bar chart of messages over time classified by their log level", `z` zoom) sebagai strip 2 baris di atas daftar: satu kolom per menit, `✖` merah di atas `▲` amber ([lnav UI docs](https://docs.lnav.org/en/stable/ui.html)). Ini sekaligus memperbaiki kelemahan TAPE hari ini — siklus runaway → 95 % → ended m.kairys yang berulang tiap menit membanjiri daftar; strip menunjukkan burst sebagai satu kolom tinggi, dan pengulangan dilipat gaya lnav: `✖ 15:26–15:29  QUERY  m.kairys on clickhouse-bi · 4× runaway → killed at 99 % · peak 8.9 GiB  ▸ 12 events`. Identifier disorot dengan bold/underline (nama node, orang) dan dim (hash, tabel) — struktur lewat bobot, bukan warna; lnav memberi warna acak pada identifier, yang dilarang aturan rumah. Mock-up ada di HOME.

**Degradasi per bentuk.** Keluarga pseudo-pixel bertingkat: block elements (universal), half-block, quadrant 2×2, braille 2×4 (hampir universal, berlubang), sextant 2×3, octant 2×4 (Unicode 16; Ghostty menggambarnya dari sprite font built-in tanpa font terpasang, Cascadia Code 2404.23 mengirimnya; Microsoft menambahkannya karena braille "makes the on-and-off pseudo-pixels unbalanced") ([ratatui Marker](https://docs.rs/ratatui/latest/ratatui/symbols/enum.Marker.html), [Ghostty Face.zig](https://git.uoc.run.place/Applied-Software/ghostty/src/commit/2a29f71b2cf6f4db0d18da9fe2fb75564f3709ea/src/font/sprite/Face.zig), [Cascadia Code 2404.23](https://devblogs.microsoft.com/commandline/cascadia-code-2404-23/)). Setting glyph tiga rung ala btop `graph_symbol` ([btop README](https://github.com/aristocratos/btop)): `octant → braille → block → ascii`, default braille, octant opt-in karena dukungan glyph tidak bisa dideteksi dari dalam app. Satu jebakan nyata: `▁–█` berlebar East Asian **Ambiguous** ([codepoints.net U+2581](https://codepoints.net/U+2581?lang=pl)); terminal yang memperlakukan ambiguous sebagai lebar (opsi umum di locale CJK) membuat setiap sparkline memakan dua sel dan merobek layout — braille dan box drawing Narrow/Neutral, jadi sparkline braille justru *lebih aman*; mitigasi: self-check saat start (cetak `▂`, query posisi kursor via `CSI 6n`) dan jatuh ke braille/ascii jika kursor bergeser dua kolom **(inferensi peneliti visualisasi)**.

| Bentuk | 24-bit | 256 | 16 warna | mono / `NO_COLOR` |
|---|---|---|---|---|
| Stacked share bar `█▓▒░·` | severity pada bar node | sama | amber → yellow bold, red → red bold | utuh — tekstur membawa segala identitas |
| Bullet bar `━┊╸▲` | ramp 3 undak | sama | 3 undak di slot ANSI | utuh — tick dan angka tetap |
| Braille/area history | area + garis cream, severity saat melewati ambang | sama | sama | utuh (tanpa warna sama sekali) |
| Heatmap ` ░▒▓█` | severity hanya pada `▓`/`█` di atas ambang | sama | sama | utuh — 5 shade |
| Gantt `━▸✔✖◌↻▪` | glyph severity + warna | sama | sama | utuh |
| Kalender jam | shade | sama | sama | utuh |
| Density strip | `✖` merah, `▲` amber | sama | sama | dua baris berlabel — utuh |
| Painted panel (Textual `panel`) | satu langkah di atas surface | langkah 256 terpilih | **hilang** → kembali ke title-rule | hilang |
| Octant sparkline | butuh font/terminal; tofu jika tidak | sama | sama | sama |
| Raster (ratatui-image) | kitty/iTerm2/Sixel | sama | sama | half-block 2 warna |

Mono sudah dilayani glyph severity `✖ ▲ ✔ ●` hari ini; yang perlu ditambah hanyalah bahwa heatmap dan stacked bar memakai `░▒▓█` agar ikut selamat. Untuk keamanan buta warna "by construction": karena warna tidak pernah membedakan dua hal yang *simultan* (hanya "seberapa buruk"), deuteranope hanya kehilangan pop pre-attentive; untuk mempertahankan pop itu, pilih merah dan hijau Tarum dari sisi Okabe–Ito (vermillion `#D55E00` daripada merah murni, bluish-green `#009E73` daripada hijau murni), yang tetap terpisah di simulasi protan dan deutan ([Okabe–Ito reference](https://dev.conceptviz.app/fr/blog/okabe-ito-palette-hex-codes-complete-reference)). Uji terang/gelap: konversi kedua tema ke grayscale; jika tiga undak severity tidak monoton dalam L*, ramp gagal untuk seseorang **(inferensi peneliti visualisasi)**. Raster inline (`ratatui-image 11.1.0`) hanya untuk artefak — flame graph PNG, ekspor chart 24 jam — bukan layar hidup: Apple Terminal "does not support Sixel or image/Terminal Graphics Protocol", dan di xterm.js shell ia jatuh ke half-block ([ratatui-image README](https://github.com/ratatui/ratatui-image), [Terminal Trove kitty vs Terminal.app](https://terminaltrove.com/compare/terminals/kitty-vs-mac-terminal/)).

## Fitur baru: history SQLite dulu, sisanya menyusul

Dua catatan riset (fitur baru dan observability ClickHouse) bertemu pada satu kesimpulan: kesenjangan terbesar cobserve terhadap monitor read-only terbaik adalah **history** (4 menit sparkline versus jam/hari dari `system.query_log`/`metric_log`), **jalur tulis ClickHouse yang tak terlihat** (parts, merges, mutations, replication, inserts — yang sesungguhnya membangunkan on-call), dan **forensik per query** yang dijadikan standar oleh chdig. Tidak ada TUI yang melakukan time-travel pada dashboard hidup; preseden terdekat adalah hwatch (setiap poll disimpan, panah berjalan di history, tiga mode diff, `--logfile` JSON dimuat ulang) dan chdig (`--start/--end` atas `system.*_log` termasuk tabel yang dirotasi) ([hwatch README](https://github.com/blacknon/hwatch), [chdig README](https://github.com/azat/chdig)). Menyimpan snapshot yang sama ke SQLite selama berjam-jam dan menambah scrubber adalah fitur paling "diam-diam tak tergantikan" dalam daftar, dan membuka sepertiga daftar lainnya **(inferensi peneliti fitur)**.

Konvensi tabel: *kelas* QI = quietly indispensable, WOW = wow, WS = wow but shallow / tidak sekarang; *effort* S = 1–2 hari, M = seminggu, L = beberapa minggu (penilaian peneliti, bukan ukuran); *tulis* menandai setiap tulisan ke sistem mana pun — ClickHouse tetap read-only tanpa pengecualian; tulisan lokal (SQLite, TOML) ditandai sebagai "lokal" agar lengkap.

| # | Fitur | Kelas | Effort | Sumber data | Tulis? |
|---|---|---|---|---|---|
| A1 | **History SQLite + scrubber** `[`/`]`, `t` lompat waktu; masthead `● live` → `◷ 03:02 · replay` | QI | M | setiap poll (NodeSnapshot, queue, tape) → `~/.local/state/cobserve/history.sqlite`, `rusqlite 0.40.2` bundled, WAL, retensi 24 h | lokal |
| A2 | Diff dua instan (`d` dalam replay): baris yang bergerak > X GiB, query `+`/`−` | QI | S setelah A1 | A1 | — |
| A3 | Sparkline `1m/10m/1h/6h` dari history | QI | S setelah A1 | A1 | — |
| E1 | **Fingerprint query**: `usually 42 s · p95 1m50 · ran 318× / 7 d · ✕ 12 failed` di drawer | QI | S | `merge('system','^query_log')` by `normalized_query_hash`, `type='QueryFinish'`, dibatasi `event_date`; poll lambat (30–60 s) atau on-demand, bukan loop 1,5 s ([diagnose guide](https://clickhouse.com/docs/guides/clickhouse/performance-and-monitoring/diagnose-slow-queries)) | — |
| H0 | **Health row per node**: parts/partition, delayed/rejected inserts, readonly replica, lag, Keeper session, disk free, uptime < 180 s | QI | S | `system.asynchronous_metrics` (`MaxPartCountForPartition`, `ReplicasMaxAbsoluteDelay`, `Uptime`), `system.metrics` (`ReadonlyReplica`, `DelayedInserts`, `ZooKeeperSession`), `system.disks`; denominator dari `system.merge_tree_settings` (`parts_to_delay_insert` 1000, `parts_to_throw_insert` 3000) ([asynchronous_metrics](https://clickhouse.com/docs/operations/system-tables/asynchronous_metrics), [merge-tree-settings](https://clickhouse.com/docs/operations/settings/merge-tree-settings)) | — |
| H1 | Merges/mutations bernama (pecah baris "server · caches · merges") | QI | S | `system.merges`, `system.mutations WHERE is_done = 0` | — |
| H2 | Failed queries panel (exception code, siapa, dari Redash query mana) | QI | S | `query_log` type Exception*, `system.error_log` (24.8+) | — |
| B3+B4 | **Catatan tape** (`n`) + **ekspor laporan insiden** (`y`): markdown dengan setiap persentase bersama denominatornya → OSC 52 + `~/incidents/…md` | QI | S | memori + A1 | lokal + clipboard |
| C2 | **Snooze / maintenance window** (`z`: 15 m / 1 h / sampai Subuh besok; `maint.toml`) | QI | S | day line sudah tahu waktunya | lokal |
| C3 | Notifikasi native terminal OSC 9/777/99 saat di luar desktop shell (tembus tmux/SSH) | QI | S | `$TERM_PROGRAM` ([tnotify](https://pkg.go.dev/github.com/soloterm/tnotify@v0.1.6)) | — |
| H1x | **OSC 8 link di setiap baris** (job → Redash, DAG → Airflow, tiket → Jira, node → Grafana) | QI | S | ada; `y` tetap fallback | — |
| J3 | Hot-reload config/rules/themes via `notify` | QI | S | `~/.config/cobserve/` | — |
| J4 | Profil `prod / rig / fr13` (`:ctx` k9s), nama profil di masthead | QI | S–M | config | — |
| E2 | **Cost hint** `EXPLAIN ESTIMATE` → `would read 1.9 B rows · 412 parts`; `EXPLAIN PLAN indexes=1` | QI | S | EXPLAIN tidak mengeksekusi dan berjalan di `readonly=1` ([EXPLAIN](https://clickhouse.com/docs/sql-reference/statements/explain)) | — |
| I1 | **"Explain this" ke sesi Claude** (`e` lalu `⏎`): prompt dari angka drawer + 30 baris tape + SQL + E1 + E2, bisa dibaca sebelum dikirim, `AI_REDACT=1` | QI | S | sesi view 5 sudah ada | — |
| K1 | Redash schedule health: failing (`schedule_failures > 0`), overdue (`2**failures` back-off), terberat | QI | S–M | `/api/admin/queries/outdated`, `/api/queries` ([redash admin.py](https://github.com/getredash/redash/blob/master/redash/handlers/admin.py), [models](https://raw.githubusercontent.com/getredash/redash/master/redash/models/__init__.py)) | — |
| K2 | Airflow "took longer than usual" `▲ 2.8× usual (median 12m)` ≥ 5 sampel; deadline 3.1 | QI | S / M | run yang sudah dibaca → A1; Airflow 3.1 Deadline Alerts ([AIP-86](https://cwiki.apache.org/confluence/spaces/AIRFLOW/pages/323488182/AIP-86+Deadline+Alerts+Formerly+SLA)) | — |
| D3 | Anomaly glyph `≈` dengan baseline tercetak `mem 61 % · usual 35 ± 8 %`; z > 3 selama ≥ 3 poll, per jam-yang-sama 7 hari | QI | M (setelah seminggu A1) | A1; prinsip konsensus Netdata ([Netdata ML](https://learn.netdata.cloud/docs/agent/ml)) | — |
| F4 | Global search / palette (channel nodes · people · queries · jobs · DAGs · tickets · sessions · tape) | QI | M | snapshot yang sudah ada, `nucleo` | — |
| G1 | Plugin gaya k9s `plugins.toml` (`$NODE $QUERY_ID $PERSON $DAG_ID $TICKET`, output `popup \| session \| none`) | QI | S–M | ([k9s plugins](https://k9scli.io/topics/plugins/), [lazygit custom commands](https://github.com/jesseduffield/lazygit/blob/master/docs/Custom_Command_Keybindings.md)) | **shell milik user** — app read-only, shell tidak |
| F2 | Heatmap fleet 60 menit (ada di HOME) | WOW | S setelah A1 | A1 | — |
| J2 | Wall mode `W` (`tui-big-text 0.8.10`, denominator di teks normal di bawahnya) | WOW | S | — | — |
| D1 | Compare dua node berdampingan (replica pair) | WOW | S | — | — |
| D2 | Overlay queue waiting vs fleet mem/cpu, satu chart 1 jam | WOW | S setelah A1 | A1 | — |
| J1 | Theme file `~/.config/cobserve/themes/*.toml` dengan slot severity wajib | WOW | S | — | — |
| E3 | Flamegraph dari `system.trace_log` | WOW | M–L | butuh `allow_introspection_functions=1`, setting user-level yang **tidak bisa** diset user `readonly=1`; degradasi ke ekspor collapsed stacks | — |
| C1 | Watch rules `rules.toml` (`when = "node.mem_pct > 85 … for 2m"`) | WOW | M | sampler sebagai model ([sampler](https://github.com/sqshq/sampler)) | — |
| C4 | Webhook post ke chat | WS | S | — | **TULIS ke chat** — opt-in per rule, tidak pernah otomatis |
| K4 | Log time dari app | WS | S | `POST /rest/api/2/issue/{key}/worklog` | **TULIS ke Jira** — off by default, konfirmasi `y` |
| H3 | `cobserve serve --ssh :2222` (russh) | WS | M–L | ([sshui](https://docs.rs/sshui/latest/sshui/)) | **membuka listener jaringan** — off by default |
| G3 | Scripting Lua/WASM | WS | L | — | — |
| H4 | Web twin via ratzilla | WS | — | kehilangan PTY sessions, yang justru alasan shell ada | — |

Satu-satunya tulisan yang *ada* hari ini — cancel job Redash (`x` lalu `y`) — tetap di tier "moderate" clig ("larger changes, deletions, remote operations: usually prompt") dan tidak ditambah typed confirmation, yang untuk delete ireversibel ([clig.dev](https://clig.dev/)). Flag `--readonly` ala k9s menonaktifkan `x` untuk orang yang tidak boleh membatalkan ([k9s README](https://github.com/derailed/k9s#readme)). Karena ClickHouse kini merekam `client_agent` ("AI coding agent detected (e.g., claude-code)") di `system.processes`, sesi Claude cobserve sendiri akan terlihat di pohon; UI harus memberi label baris itu agar investigasi tidak mengejar dirinya sendiri ([processes docs](https://clickhouse.com/docs/operations/system-tables/processes)). Pola AI yang terbukti di 2025–26 (Grafana Assistant Investigations, Datadog Bits, ClickHouse Ask AI) sama: agen membaca data read-only yang sama dengan UI, membentuk hipotesis, dan mengembalikan laporan terstruktur yang bisa diarahkan manusia — "No DDL auto-application; recommendations remain advisory only" ([chmonitor README](https://github.com/duyet/clickhouse-monitoring), [Grafana what's new](https://grafana.com/whats-new/2026-07-29-assistant-investigations-is-now-generally-available-in-grafana-cloud/)); cobserve sudah memiliki bagian tersulitnya (Claude di terminal, SQL read-only), yang hilang hanyalah hand-off terstruktur I1. Gimmick untuk dihindari: chat pane selalu-nyala, "AI insights" yang menduplikasi insight deterministik — rumus §5 tetap sumber kebenaran.

## Kemudahan pakai: navigasi, kunci, umpan balik, aksesibilitas

**Model navigasi** adalah tumpukan: `home › fleet › nodes › clickhouse3 › query 8f1c`, setiap `⏎` mendorong, `esc` mencabut satu, `⌫` sama, `-` dan `[`/`]` berjalan di riwayat seperti k9s ("comparable to browser history or `cd -`") ([k9s README](https://github.com/derailed/k9s#readme)). Satu aturan `esc` yang ditulis: menutup hal teratas (palette, filter, drawer, floating pane, sub-view), tidak pernah keluar, tidak pernah "mengibaskan pengingat shalat" — itu pindah ke dismiss toast. `+` memutar densitas *comfortable → compact → focus* (lazygit `screenMode`), disimpan di config: compact melipat band menjadi satu baris dan day line menjadi chip 14 kolom di masthead `● Maghrib in 1h58m`; focus hanya body + footer. Breakpoint dideklarasikan dalam satu `fn density(area) -> Density` yang ditanyakan setiap view: 80×24 satu pane, denominator di header; 120×36 target; ≥ 150 inspector menjadi kolom kanan. Jump label ala Helix/posting (`f` lalu dua huruf di atas setiap baris) menutup kasus "saya melihatnya, biarkan saya mendarat di sana" yang tidak dijawab `/` — berharga di MAP 40 node ([posting guide](https://posting.sh/guide/), [Helix editor config](https://docs.helix-editor.com/editor.html)). Mouse diharapkan pada 2026 (k9s, btop, lazygit, zellij semua mendukungnya): klik tab, baris, hint footer (Textual membuat kunci footer bisa diklik), roda untuk scroll, seret batas drawer (simpan rasio seperti lazygit `sidePanelWidth`), OSC 8 pada run/tiket; `MOUSE=0` tetap untuk yang ingin seleksi terminal.

| Kunci | Arti, di mana pun | Catatan |
|---|---|---|
| `0 1 2 3 4 5` | home · fleet · queue · airflow · jira · sessions | digit tersembunyi < 100 kolom |
| `tab` / `[` `]` | sub-view (nodes · map · heat · tape) / riwayat | |
| `⏎` / `esc` `⌫` | drill / mundur satu | `esc` tidak pernah keluar |
| `/` | filter lokal, hitungan `3 of 15`, karakter cocok disorot | semua daftar, semantik sama |
| `:` atau `ctrl+p` | palette (aksi + channel pencarian) | `?` = palette yang terbuka pada scope fokus |
| `e` | explain this number | `⏎` di popover → kirim ke Claude (I1) |
| `f` | zoom region fokus / jump label | `+` densitas |
| `ctrl+\` | floating session; lalu `1–5` view, `7–9 ↑↓` sesi, `n` baru | `F1–F10` fallback |
| `x` → `y` | cancel job Redash (satu-satunya tulisan) | dialog, bukan baris footer |
| `[` `]` `t` | scrub waktu (A1) | masthead `◷ … replay` |
| `n` `z` `P` `y` | catatan tape · snooze · pin · copy/ekspor | |
| `p` `r` `q` | pause · read now · quit | |

**Footer, `?`, palette** berasal dari satu tabel aksi, seperti dibahas di atas; footer memuat maksimal lima hint untuk pane fokus, diurutkan menurut frekuensi (talk PyCon India 2024 menyebut "priority-ranked hints based on usage frequency" sebagai praktik Zulip terminal) ([PyCon India CFP](https://in.pycon.org/cfp/2024/proposals/tuis-simplified-the-transformation-of-tuis-into-user-friendly-tools~dwp2m/)), `?` dibatasi pane fokus dengan bagian gaya lazygit dan dapat difilter dengan mengetik, ≤ 12 baris di 80×24. **Empty state mengajar**: QUEUE tanpa Redis — "WAITING names need Redis · counts only — `:setup redash` adds `REDIS_URL`"; AIRFLOW belum dikonfigurasi — "No Airflow yet — `s` to connect one · `3` later" (lazygit memakai kalimat polos: "No changed files") ([lazygit english.go](https://github.com/jesseduffield/lazygit/blob/master/pkg/i18n/english.go)). **Tour pertama kali** 6–8 langkah ≤ 60 detik digambar di atas layar *hidup* dengan region relevan disorot, sentinel `~/.local/state/cobserve/tour-done`, dibuka lagi dengan `:tour` — model teamctl-ui (10 langkah, "Press any key to advance", `Esc` skip, sentinel presence-only) ([teamctl-ui onboarding.rs](https://docs.rs/teamctl-ui/0.11.0/src/teamctl_ui/onboarding.rs.html)).

**Toast, bukan notice footer**: tumpukan kanan-bawah, maksimal 3, 5 detik default dan 10 detik untuk error (default Textual `NOTIFICATION_TIMEOUT = 5`), glyph severity + warna (`✓ link copied`, `▲ airflow refused the login · retry in 4m58s · r now`, `✖ clickhouse3 · no answer within 1.5 s · CH_TIMEOUT_MS raises it`), `esc` atau klik menutup yang teratas, `:notifications` mendaftar 20 terakhir ([Textual App API](https://textual.textualize.io/api/app/)). Pengingat shalat menjadi toast juga. Footer kanan hanya memuat baris poll (`9/9 nodes · 0.8 s · read-only`). **Loading per sumber**: setiap judul view membawa kesegarannya sendiri (`AIRFLOW · read 4 s ago · reading…`) dengan spinner satu glyph yang menjadi `…` statis dalam mode aksesibel (GitHub CLI mengganti spinner dengan "a static text progress indicator"), dan tier *stale* (data lebih tua dari 3 poll) digambar redup dengan `· 47 s old`, tidak pernah membeku diam-diam ([GitHub blog](https://github.blog/engineering/user-experience/building-a-more-accessible-github-cli/)). **Kata-kata error** = apa yang terjadi + mengapa + kuncinya: "Jira refused the token (401) — `:setup jira` to paste a new one · `r` to retry"; tidak pernah string error Rust di pane (clig: "Catch errors and rewrite them for humans") ([clig.dev](https://clig.dev/)). **Dialog cancel** (yazi `[confirm]`: judul, badan, dua tombol): judul `Cancel Redash job #8213?`, badan tiga fakta yang sudah dicantumkan GUIDE — siapa, query mana, "its query on clickhouse3 keeps running; cobserve never sends KILL QUERY" — plus "This cannot be undone; the job can be run again from Redash", tombol `[y] cancel the job   [esc] keep it` ([yazi theme](https://yazi-rs.github.io/docs/configuration/theme)).

**Aksesibilitas** mengikuti praktik yang *dikirim* GitHub CLI 2025–26: "align our color palettes to 4-bit colors so our users can completely customize their experience using their terminal preferences" ([GitHub blog](https://github.blog/engineering/user-experience/building-a-more-accessible-github-cli/)). Untuk cobserve: severity default ke 16 slot ANSI (red/yellow/green/blue), palet 24-bit Tarum menjadi lapisan *aksen* saja; `NO_COLOR` → varian mono, `TERM=dumb`/`force_tty` → glyph polos; tema terang dipilih dari latar terminal yang dilaporkan (query OSC 11) dengan override `THEME=`; `MOTION=off` menjadikan spinner statis dan mematikan efek `tachyonfx` bila diadopsi (atuin `prefers_reduced_motion` sebagai preseden) ([atuin config](https://github.com/atuinsh/atuin/blob/main/crates/atuin-client/config.toml)). Screen reader: jangan menjanjikan TUI aksesibel — kritik yang ada menyebut TUI kanvas "actively spams you" dan menganjurkan "a dumb, linear CLI stream" ([xogium.me](https://xogium.me/the-text-mode-lie-why-modern-tuis-are-a-nightmare-for-accessibility)); jawaban jujurnya adalah `cobserve --plain` yang mencetak daftar INSIGHTS dan band sebagai teks linear setiap poll — yang juga diinginkan pengguna SSH tanpa tmux dan pager. Tidak ada ketergantungan Nerd Font (default lazygit `nerdFontsVersion: ""` = tanpa ikon); logo `❖` keluar dari tier fallback.

## Stack ratatui 0.30 dan matriks terminal

ratatui 0.30 (kini 0.30.2, workspace `ratatui-core 0.1.2 / ratatui-widgets 0.3.2 / ratatui-crossterm 0.1.2`, Rust 2024, MSRV 1.86) adalah rilis modular; yang relevan untuk redesign: `merge_borders(MergeStrategy)` untuk grid kartu tanpa artefak garis ganda, `BorderType` dashed dan quadrant, `Block::shadow()`, `Rect::layout()`/`centered()`/`outer()`, `Flex::SpaceEvenly`, marker `Quadrant/Sextant/Octant`, `LineGauge::filled_symbol()`, `Text += Text`, `ratatui::run()`, dan `Table` dengan `row/column/cell_highlight_style` ([ratatui 0.30 highlights](https://ratatui.rs/highlights/v030/), [Table docs](https://docs.rs/ratatui/latest/ratatui/widgets/struct.Table.html)). Hampir seluruh ekosistem sudah di 0.30 per crates.io 2026-10-13; frameworks (tuirealm 4.1.0, rat-salsa 4.0.3) tidak diperlukan untuk app sebesar cobserve dan akan memaksa penulisan ulang event handling ([crates.io via catatan design-language](https://crates.io/crates/tuirealm)).

| Crate | Versi (tanggal) | Peran | Catatan |
|---|---|---|---|
| ratatui | 0.30.2 (2026-09-10) | inti; Chart Area/Braille/Octant, Sparkline absent values, BarChart grouped, Gauge/LineGauge, Calendar, Fill | sudah dipakai |
| crossterm | 0.29 | backend; `KeyboardEnhancementFlags`, `Begin/EndSynchronizedUpdate`, `osc52` feature | sudah dipakai; emit DEC 2026 sendiri di sekitar `terminal.draw` sampai dikonfirmasi backend melakukannya |
| tokio, reqwest 0.12, serde, portable-pty 0.9, vt100 0.16 | sekarang | poller, HTTP, PTY | **pertahankan** |
| tui-input | 0.15.5 (2026-09-26) | baris `:` dan `/` | |
| nucleo / nucleo-matcher | pin saat build | fuzzy palette & finder (matcher Helix, dipakai television) | versi tidak tercatat di catatan |
| ratatui-textarea | 0.9.3 (2026-10-05) | editor SQL multi-baris (fork org ratatui) | **bukan** `tui-textarea 0.7.0` (2024-10-22, ratatui ^0.29 — basi) |
| tui-tree-widget | 0.24.1 (2026-08-09) | pembanding untuk pohon node→user→query sendiri | ukur, tidak wajib |
| tui-scrollview 0.6.8, tui-popup 0.7.7, tui-prompts 0.6.8, tui-big-text 0.8.10, tui-scrollbar 0.2.8 | 2026-09-24 (monorepo `ratatui/tui-widgets`) | inspector panjang, popup, wizard, wall mode | rilis lockstep |
| tui-logger | 0.18.3 (2026-07-04) | debug mode TAPE | |
| ansi-to-tui | 8.0.1 (2026-01-10) | log task Airflow berwarna | |
| tui-markdown | 0.3.11 (2026-10-10) | deskripsi Jira, jawaban Claude | |
| keyring | pin saat build | Keychain / Secret Service / Credential Manager | versi tidak tercatat di catatan |
| rusqlite | 0.40.2 (2026-10-12), `bundled` | history A1 | DuckDB ditolak: build C++ berat, kekuatan kolumnar tidak relevan pada skala ini |
| toml + serde, directories/etcetera, notify | pin saat build | config.toml XDG, hot-reload | |
| termprofile | 0.2.4 (2026-05-02) | deteksi kedalaman warna | ganti deteksi 24-bit/256/16 buatan sendiri |
| hyperrat | 0.1.3 (2026-06-28) | widget OSC 8 (membungkus trik 2-karakter #902) | atau tulis sendiri ~30 baris |
| tachyonfx | 0.25.2 (2026-09-06) | efek fade untuk toast/palette, *opsional*, mati di `MOTION=off` | efek = pasca-proses Buffer, butuh redraw per frame |
| ratatui-image | 11.1.0 (2026-10-05) | artefak saja (flame graph PNG, chart 24 h) | Alacritty/Konsole/Warp lemah; Sixel baris akhir menggulung |
| malevich | 1.24.1 (2026-10-03) | heatmap (evaluasi) | |
| catppuccin 2.8.0 / opaline 0.4.2 | 2026 | model token semantik jika Tarum menjadi *sistem* tema | opaline: 30k unduhan, muda |

**Hindari**: `tui-textarea 0.7.0` (ratatui ^0.29, tidak compile di 0.30 tanpa patch); `flamelens 0.4.0` (ratatui 0.29 — vendor renderer-nya, jangan depend); `ratatui-command-palette 0.1.0` (experimental — pelajari); crate toast/overlay/skeleton 0.1.x (`tui-overlay`, `tui-skeleton`, `ratatui-toaster`; < 5k unduhan, 50–200 baris — implementasi sendiri di dalam tema); `modalkit 0.0.31` (API pre-1.0, dependency besar); `tui-piechart` (melanggar aturan warna); `tui-bar-graph` dengan gradien `colorgrad` (gradien hue bertabrakan dengan "warna = severity" kecuali monokrom); `plotters-ratatui-backend` **tidak ada** di docs.rs/crates.io (dua pencarian) — satu-satunya jalur Plotters adalah render ke RGB buffer (`plotters-bitmap`) lalu ratatui-image ([plotters-backend](https://docs.rs/plotters-backend/)); `trazo 0.0.1` pantau saja.

Matriks terminal berikut menggabungkan sumber ketiga catatan; sel bertanda **?** tidak diverifikasi dan diturunkan dari kolom lain **(inferensi)**:

| Terminal | Truecolor | Kitty keyboard | OSC 8 link | OSC 52 copy | DEC 2026 sync | Gambar inline | Octant |
|---|---|---|---|---|---|---|---|
| Ghostty | ya | ya | ya | **?** (tidak di daftar go-osc52) | ya | Kitty (unicode placeholders) | **ya, sprite built-in** |
| kitty | ya | ya | ya | ya | ya | Kitty ≥ 0.28 | font-dependent |
| WezTerm | ya | ya | ya | ya | ya | iTerm2 (satu-satunya yang bebas bug di sana) | font-dependent |
| iTerm2 | ya | ya | ya | ya | ya | iTerm2 | font-dependent **?** |
| Apple Terminal | **?** (umumnya 256) | tidak terdaftar | **?** | **?** | **?** | **tidak** | **?** |
| VS Code (xterm.js) | ya | ya (xterm.js) | ya **?** | **?** | **?** | Sixel/iTerm2 hanya jika `terminal.integrated.enableImages` | font-dependent |
| tmux | ya (passthrough) | **?** passthrough | **?** | **?** (zellij 0.45+ memblokir *baca*, tulis boleh) | diabaikan | half-block (asumsi) | font-dependent |
| wry + xterm.js (desktop) | ya | ya (xterm.js) | **? — periksa versi di `desktop/`** | **?** | **?** | **tidak** tanpa addon | font-dependent |

Sumber: daftar implementasi protokol kitty ([kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)); OSC 52 ([go-osc52](https://pkg.go.dev/github.com/aymanbagabas/go-osc52)); DEC 2026 "Ghostty, Kitty, Alacritty, WezTerm, iTerm2, Windows Terminal" dan terminal tanpa dukungan "just ignore the escapes" ([ratto docs, sekunder](https://docs.rs/ratto/0.7.0/rat/)); gambar ([ratatui-image README](https://github.com/ratatui/ratatui-image), [yazi image preview](https://yazi-rs.github.io/docs/image-preview), [Petri on VS Code 1.80](https://petri.com/visual-studio-code-1-80/)); Apple Terminal tanpa gambar ([Terminal Trove](https://terminaltrove.com/compare/terminals/kitty-vs-mac-terminal/)). Konsekuensi desain: **probe kapabilitas saat start** (termprofile untuk warna, `supports_keyboard_enhancement`, `Picker` ratatui-image untuk grafik, `$TERM_PROGRAM` untuk heuristik OSC 8) memilih painted panel vs rule, octant vs braille vs `▂▃▄`, link klik vs `y` copy — dan **shell desktop milik owner sendiri adalah tempat pertama jalur raster gagal**, satu alasan lagi untuk tidak menaruh raster di layar hidup **(inferensi peneliti visualisasi)**. Alternatif xterm.js untuk shell desktop — ratzilla (DOM/Canvas/WebGL2) atau backend wgpu/soft — kehilangan sesi PTY yang justru alasan shell itu ada; tidak direkomendasikan ([ratzilla](https://docs.rs/ratzilla/latest/ratzilla/)).

## Rekomendasi: bangun ulang shell, pertahankan mesin — dalam delapan fase

**Keputusan: evolusi radikal, bukan rewrite repo.** Argumennya kuantitatif dan arsitektural. Dari 44.006 baris Rust, lapisan yang menentukan konsep UI — `src/ui/*` (~12 ribu) plus state navigasi di `src/app.rs` (3.798) — sekitar sepertiga; dua pertiga sisanya (`model.rs` dengan §5, `sources/*`, `history.rs`, `insight.rs`, `tape.rs`, `pty.rs`, `claude.rs`, `console.rs`, `prayer.rs`, `theme.rs`, `fake.rs`, `config.rs`, uji) adalah *mesin* yang tidak peduli layar mana yang menggambarnya **(inferensi dari `wc -l src/**/*.rs`)**. Aturan "UI never blocks on network" sudah memaksa pemisahan poller dari UI, yang berarti shell baru dapat dibangun *berdampingan* di balik flag (`UI=2` atau `cobserve --shell next`), diberi makan snapshot yang sama, dan diadu dengan shell lama pada rig lokal sampai angkanya identik — yang pasti terjadi karena rumusnya satu. Menulis ulang repo dari nol akan membuang §5 yang teruji, generator fake yang menjadi dasar `dev/screenshots.sh`, dan tiga kuartal pengetahuan domain di `sources/clickhouse.rs` (2.116 baris berisi gotcha §12 seperti `is_initial_query = 1` dan fallback `server_cpu_time_us`) tanpa manfaat apa pun pada konsep UI. Sebaliknya, *menambal* shell lama tidak akan menghasilkan "konsep berbeda": HOME, grammar resource, crumbs, palette, floating session, dan densitas adalah struktur, bukan ornamen, dan `app.rs` 3.8 ribu baris yang sudah membawa sepuluh view sebagai enum adalah tempat yang salah untuk menumbuhkannya. Jadi: **shell UI dari nol, mesin tetap, dan keduanya bertemu lewat dua kontrak baru** — tabel `Action` dan struct `Provenance` — yang ditambahkan ke mesin sebelum satu baris shell baru ditulis.

Yang **dipertahankan tanpa tawar**: matematika §5 dan invarian §5.3 yang diuji; warna hanya severity; setiap persentase mencetak denominatornya (di 80 kolom lewat header kolom); setiap angka terlacak ke rumus (kini lewat `e`); read-only pada ClickHouse termasuk `EXPLAIN` yang berjalan di `readonly=1`; palet Tarum (soga/mori/tarum) sebagai token semantik `ground/text/accent/panel/severity.{red,amber,green}` dengan `panel = Color::Reset` di mono; day line shalat sebagai baris kedua permanen (chip di densitas compact); sesi PTY dengan `portable-pty`+`vt100` dan `ctrl+\`; insight engine sebagai sumber HOME; ring buffer history 4 menit dan tape; tier cancel `x`→`y`; `y` copy-link sebagai fallback OSC 8; `FAKE=1`, rig lokal, dan pipeline screenshot sebagai acceptance. Deviasi §13 yang sudah diputuskan (baris per (user, person), `QueryStat::cores`, `unreachable_reason`, login form Airflow) tetap berlaku.

Urutan fase, fake data dulu seperti §11, setiap fase punya acceptance berupa ASCII di laporan ini:

**Fase 0 — kontrak di mesin (S).** Tambah `Action { id, label, keys, scope, enabled: fn(&App) -> bool, run }` dan `Provenance { formula_id, inputs, at }` pada setiap metrik §5; `fn density(area) -> Density`; probe kapabilitas (`termprofile`, `supports_keyboard_enhancement` sebelum event loop, `$TERM_PROGRAM`); emit `Begin/EndSynchronizedUpdate` di sekitar `draw`. Tidak ada perubahan layar. Uji: provenance setiap angka di snapshot fake dapat direkonstruksi ke nilainya (±0,1).

**Fase 1 — kerangka shell + HOME (M).** `FAKE=1 UI=2 cargo run` menampilkan masthead, day line, crumbs line, band satu baris, footer lima hint, HOME dengan empat kartu + daftar "what's wrong now" lintas sumber + density strip + heatmap (dari ring buffer 4 menit dulu); tumpukan `esc`; palette (`tui-input` + `nucleo`) dan `?` sebagai palette ber-scope; toast. Acceptance: mock-up HOME 120×36 di atas, serta 80×24 dan 160×48 tidak panic (uji resize fuzz yang sudah ada diperluas ke shell baru).

**Fase 2 — FLEET sebagai resource (M).** Tabel node→user→query dengan stacked share bar (largest-remainder), bullet bar, inspector drawer/kolom menurut densitas, `f` zoom, sub-view nodes · map · heat · tape pada `tab`, `/` dengan hitungan, jump label. Acceptance: mock-up FLEET 120×36 dan 160×48; invarian §5.3 terlihat sebagai bar yang penuh tanpa sel nyasar.

**Fase 3 — QUEUE, AIRFLOW, JIRA pada grammar yang sama (M).** Masing-masing menjadi tabel resource + inspector + crumbs; Gantt per DAG terpilih, kalender jam Jira, dialog cancel. Acceptance: setiap layar `docs/screenshots/120x36-*.txt` hari ini punya padanan di shell baru dengan angka yang sama.

**Fase 4 — sesi (S–M).** `5 sessions` sebagai resource list, floating pane `ctrl+\`, which-key setelah 250 ms, keyboard enhancement flags dengan fallback `F1–F10` + toast. Acceptance: mock-up sesi; uji `live_airflow_and_jira`-style untuk prefix di Ghostty/iTerm2/xterm.js dicatat sebagai `--ignored`.

**Fase 5 — explain, wizard, plain (M).** Popover `e` dari `Provenance`; setup wizard dengan `keyring`, fallback 0600 yang diumumkan, `config.toml` XDG, env override, `d` demo; `--plain`; `MOTION=off`; `NO_COLOR`. Acceptance: mock-up wizard dan explain; `cobserve --plain` mencetak INSIGHTS + band setiap poll.

**Fase 6 — data nyata dan pergantian (S–M).** Jalankan kedua shell terhadap `dev/local-rig.sh` lalu fleet; bandingkan angka (harus identik — mesin sama); perbarui `dev/screenshots.sh` ke layar baru; hapus shell lama; DESIGN.md baru lahir dari laporan ini dengan §1 (layar), §3 (kunci), §7 (rendering), §11 (urutan) diganti dan §5, §6, §9→"environment-first", §12 dipertahankan.

**Fase 7 — fitur gelombang 1 (M–L total).** A1 history SQLite + scrubber (membuka A2, A3, F2, D2); E1 fingerprint; H0 health row + H1 merges + H2 failed queries sebagai *poll class kedua* (30–60 s) yang dirancang sekali untuk semua tabel `*_log`; B3+B4 catatan + ekspor; C2 snooze; C3 notifikasi native; OSC 8; J3 hot-reload; J4 profil; E2 cost hint; I1 explain → Claude.

**Fase 8 — gelombang 2 (M–L).** K1 Redash schedule health; K2 Airflow "took longer than usual"; D3 anomali setelah seminggu A1; G1 plugin; J2 wall mode; J1 theme file; D1 compare; E3 flamegraph hanya jika grant introspection ternyata ada.

## Celah riset yang dibawa serta

Para peneliti mencatat kesenjangan berikut; semuanya tetap terbuka dan beberapa harus ditutup *sebelum* fase terkait. Tidak ada riset pengguna pada cobserve sendiri — "yang sulit hari ini" disimpulkan dari artefak, bukan diamati; tidak ada studi usability terkontrol untuk list+detail vs tiles, palette vs kartu `?`, tab bernomor vs palette, atau border vs borderless — rekomendasi bertumpu pada konvergensi k9s/lazygit/Helix/yazi dan pernyataan desain Textualize/posting. Layout `:pulses` k9s hanya terkonfirmasi sebagai "monitoring dashboard showing cluster health" (halaman 404); chrome sidebar/dialog Crush tidak terbaca; spesifik TUI Claude Code berasal dari sumber sekunder (ink-terminal README); palette dan footer Harlequin tidak terbaca (429). Apakah backend crossterm ratatui 0.30 sudah membungkus `draw()` dalam DEC 2026 **tidak dikonfirmasi** — periksa `ratatui-crossterm` sebelum menambah emisi manual; apakah ratatui punya dukungan OSC 8 native di luar workaround contoh tidak dikonfirmasi; README `tui-term` tidak terbaca (404 di `main`). Font macOS yang memuat Legacy Computing Supplement (JetBrains Mono, Iosevka, SF Mono, Menlo) tidak terukur — perlakukan octant sebagai Ghostty/Windows Terminal saja sampai diuji `printf '\U1CD00'`; perilaku fallback Apple Terminal dan iTerm2 untuk U+1CDxx tidak diverifikasi. Dukungan xterm.js/WebKit (wry) untuk OSC 8, mode 2026, dan grafik kitty **tidak diperiksa** terhadap versi yang dibundel `desktop/` — ini menentukan apa yang didapat Cobserve.app dan harus dicek di fase 0. Konvensi sumbu bottom (label jendela di tepi kiri) tidak bersumber (docs 404); tidak ada crate LTTB untuk ratatui; daftar backend trazo berasal dari snippet; renderer flamelens belum dibaca. Pada sisi data: apakah REST API stabil Airflow 2 mengekspos SLA misses tidak ditemukan; apakah `/api/queries` Redash menyertakan `schedule_failures` tidak dikonfirmasi (terdokumentasi sebagai kolom DB); `system.query_metric_log` diyakini 24.11 dan mungkin absen di node 24.10 (`EXISTS TABLE`); `part_log`, `query_views_log`, `text_log`, `processors_profile_log` harus diaktifkan di server dan status di fleet tidak diketahui; apakah fleet Paysera memakai Kafka engine, S3Queue, Distributed, dictionaries, atau BACKUP tidak ada di konteks; `allow_introspection_functions` pada profil read-only tidak diverifikasi (menentukan nasib E3); endpoint worklog Jira tidak diambil. Tidak ada benchmark beban insert SQLite dari app ratatui di laptop (estimasi A1 adalah aritmetika); sampler terakhir rilis ~v1.1.0 dengan status pemeliharaan tak terverifikasi; `evalexpr` vs `rhai` untuk C1 tidak dibandingkan. Semua estimasi effort adalah penilaian, bukan ukuran; kecepatan owner tidak diketahui peneliti.

## Kesimpulan

Yang berubah dari penelitian ini bukan jawaban "bagaimana membuat cobserve lebih cantik" — cobserve sudah berada di sisi yang benar dari setiap konvensi chrome 2024–2026 — melainkan pemahaman bahwa kekuatannya yang paling khas belum pernah menjadi *fitur*: aturan "setiap angka terlacak ke rumus" selama ini adalah disiplin penulis, dan popover `e` mengubahnya menjadi sesuatu yang dipakai pengguna setiap hari, sekaligus alat paritas dengan FleetLens dan prompt yang siap dikirim ke Claude. Hal yang sama berlaku untuk "warna = severity": aturan yang terdengar seperti pembatasan ternyata memaksa setiap visualisasi baru bertekstur, dan tekstur adalah tepat apa yang membuat stacked bar, heatmap, dan Gantt bertahan di mono, di tmux, dan di shell desktop yang tidak bisa menggambar raster — cobserve mendapat portabilitas sebagai efek samping dari kejujuran.

Implikasinya untuk DESIGN.md berikutnya: dokumen itu harus memulai dari HOME dan grammar resource, bukan dari view 1, dan menambahkan dua kontrak — `Action` dan `Provenance` — ke §4 sebelum §5, karena keduanya adalah bagaimana footer, `?`, palette, dan `e` tetap konsisten tanpa upaya. Dan history SQLite bukan "fitur nomor satu" karena paling berkilau, melainkan karena ia mengubah pertanyaan on-call dari "apa yang terjadi sekarang" menjadi "apa yang terjadi pukul 03:02, apakah itu normal untuk Senin, dan apa yang berubah" — tiga pertanyaan yang hari ini dijawab Redash plus notebook, dan besok dijawab `[`, `≈`, dan `d`.
