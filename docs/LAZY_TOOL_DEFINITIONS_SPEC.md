# Spec: Static Tiering for Builtin Tool Definitions (Lazy-Loading Opsi A)

> Status: **Draft untuk diskusi — Jangan eksekusi sebelum approval.**
> Tanggal: 2026-11-05
> Terkait: diskusi lazy-loading tools pada sesi `session_stallion_179`, VCC compaction

## 1. Ringkasan

Bagi builtin tools jcode menjadi dua **tier statis**:

- **Tier 1 — Eager** (~10 tools, ~3K token): selalu dikirim sebagai definisi penuh dalam prompt prefix.
- **Tier 2 — Lazy** (~19 tools, ~9K token): *tidak* dikirim di prefix. Hanya satu baris ringkas di system prompt yang mengumumkan keberadaannya. Definisi penuh dimuat on-demand lewat mekanisme yang sudah ada untuk MCP (`defer_loading` / search-call surface).

Target: prefix turun dari **18,279 token** (`system=5,871 tools=12,408` per `~/.jcode/logs/jcode-2026-11-05.log`) menjadi **~9–10K token**.

## 2. Latar belakang & bukti

| Fakta (terverifikasi di repo) | Lokasi |
|---|---|
| 29 tools dikirim eager; `apply_mcp_tool_exposure()` hanya me-lazy-kan `mcp__*` | `crates/jcode-app-core/src/agent/turn_execution.rs:701` |
| `ToolDefinition.defer_loading: bool` sudah ada; `native_deferred_tool_definitions()` memisahkan eager vs deferred | `turn_execution.rs:622`, `crates/jcode-message-types/src/lib.rs` |
| `TOOL_REFERENCES_METADATA_KEY = "tool_references"` — output tool bisa me-load definisi deferred via referensi | `crates/jcode-app-core/src/agent/tools.rs:80` |
| `ToolConfig` sudah punya `profile`/`enabled`/`disabled`/`codemode` — pola opt-in config yang sama dipakai di sini | `crates/jcode-base/src/config.rs:840+` |
| Locked snapshot per sesi (`locked_tools`) + one-shot late-MCP check (#206) — tiering harus *tidak* mengubah snapshot mid-session | `turn_execution.rs:501–604` |
| `codemode` sudah lazy secara selektif (tidak diregistrasi jika `codemode=false`) | `crates/jcode-app-core/src/tool/mod.rs:569` |

### Estimasi penghematan

| Tier | Tools | Estimasi token |
|---|---|---|
| Tier 1 (eager) | `read`, `write`, `edit`, `apply_patch`, `bash`, `todo`, `swarm`, `batch`, `bg`, `agentgrep` | ~3,000 |
| Tier 2 (lazy) | `browser`, `calendar`, `gmail`, `memory`, `maintainer_feedback`, `compile_remote`, `panel`, `side_panel`, `applet`, `open`, `conversation_search`, `session_search`, `skill_manage`*, `schedule`, `codemode`, `jcode_docs`, `selfdev`, `webfetch`, `websearch`, `desktop_selfdev`, `debug_socket`, `integration_tools` | ~9,400 |
| Meta-tool `tools` (search/call) | 1 tool pengganti | ~300 |
| **Total baru (prefix tool)** | | **~3,300 (vs 12,408)** |

\* `skill_manage` tetap eager jika skill interaktif dipakai sering — lihat §5 open question.

## 3. Tujuan / Non-tujuan

**Tujuan:**
1. Prefix awal < 11K token tanpa mengubah perilaku model untuk workflow harian (edit file, bash, todo, swarm).
2. Tier 2 tetap dapat dipanggil dengan maksimal 1 langkah tambahan (satu `tools` search/call).
3. Nol prompt-cache miss tambahan setelah turn pertama (snapshot tetap locked).
4. Configurable per project/global; default `static` hanya aktif bila eksplisit di-enable.

**Non-tujuan (Opsi B/C, ditunda):**
- Usage-based promotion adaptif (Opsi B) — butuh telemetri frekuensi per tool.
- Native deferred penuh di semua provider (Opsi C) — tetap fallback ke fixed-surface transcript.

## 4. Desain

### 4.1 Config

```toml
[tools]
# "eager" (default, perilaku lama) | "static" (tier 1+2)
builtin_tiering = "eager"
# override tier per tool (opsional)
# eager_tools = ["read", "bash"]
# lazy_tools  = ["browser", "gmail"]
```

- Field baru di `ToolConfig` (`crates/jcode-base/src/config.rs`), default `Eager` agar backward-compatible.
- `builtin_tiering = "static"` mengaktifkan semua perilaku di §4.2–4.5.
- Env override mengikuti pola config yang ada.

### 4.2 Tiering di registry

Tambahkan fungsi `tier_for_tool(name: &str) -> Tier` di `crates/jcode-app-core/src/tool/` (file baru `tool_tiers.rs` atau di `mod.rs`):

```rust
pub enum Tier { Eager, Lazy }
pub fn tier_for_tool(name: &str) -> Tier { /* match name */ }
```

- Daftar Tier 1 dikunci di kode (bukan config) agar mudah diaudit dan ditest. Override hanya lewat `eager_tools`/`lazy_tools` eksplisit.
- `selfdev`/`debug_socket`/`desktop_selfdev`/`jcode_docs` selalu ikut aturan `apply_selfdev_tool_surface` yang sudah ada — tiering tidak mengubahnya.

### 4.3 Definisi deferred untuk builtin

Di `build_filtered_tool_definitions_with()` (`turn_execution.rs:664`), setelah `apply_selfdev_tool_surface` dan sebelum `apply_mcp_tool_exposure`:

```rust
if tiering_enabled {
    for tool in tools.iter_mut() {
        if tier_for_tool(&tool.name) == Tier::Lazy {
            tool.defer_loading = true;
        }
    }
}
```

### 4.4 Provider surfaces — reuse MCP path (tidak ada kode baru)

| Provider | Perilaku |
|---|---|
| Native deferred (`supports_deferred_tools()`) | Tier 2 dikirim sebagai `defer_loading=true` — sama persis dengan MCP path (`native_deferred_tool_definitions`, `turn_execution.rs:622`). Model me-load via referensi. |
| Non-native (fixed surface) | Tier 2 **tidak** dikirim; model memakai meta-tool `tools` (§4.5). Definisi dimasukkan ke transcript via mekanisme `tool_references` seperti MCP. |

**Kunci:** eager snapshot (`locked_tools`) hanya berisi Tier 1 + `tools`. Tier 2 tidak pernah masuk snapshot ⇒ tidak ada cache miss saat Tier 2 di-load.

### 4.5 Meta-tool `tools` (pengganti unified `mcp` untuk builtin)

Satu tool baru, pola yang sama dengan `mcp` unified surface:

```
tools(action="search", query="browser")        -> daftar Tier 2 yang cocok + input_schema ringkas
tools(action="call", name="browser", args={})  -> dispatch ke registry langsung
```

- Alternatif yang ditolak: menambah tool `tool_load` terpisah (menambah surface baru; `tools` reuse pola `mcp` yang sudah dipahami model).
- Jika `builtin_tiering = "eager"`, meta-tool `tools` **tidak diregistrasi** (tidak ada biaya).

### 4.6 System prompt note

Satu baris di `build_system_prompt_split` static part saat tiering aktif:

```
Lazy tools available (load with tools action=search/call): browser, calendar, gmail, ...
```

< 50 token. Di luar itu, model tidak perlu tahu apa pun tentang Tier 2.

### 4.7 Cache & lifecycle

- Snapshot di-lock sekali di turn pertama seperti sekarang (`Locking tool list at N tools`, `turn_execution.rs:589`).
- Provider switch / `unlock_tools()` me-rebuild snapshot — Tier 1 tetap sama ⇒ tidak ada miss baru.
- Late-connect MCP path tidak tersentuh.

## 5. Open questions (perlu keputusan sebelum implementasi)

1. **Apakah `skill_manage` tetap Tier 1?** Skill list adalah komponen system prompt terbesar (~2,5–3K token). Jika `skill_manage` lazy, user tidak bisa install skill dari sesi interaktif tanpa 1 langkah tambahan. Usul: tetap Tier 1.
2. **Apakah `codemode` masuk Tier 2?** Codemode bisa memanggil semua tool termasuk Tier 2 dari dalam sandbox — mungkin membuat Tier 2 tidak perlu di-load manual. Perlu uji: apakah model dengan codemode aktif masih butuh Tier 2 eager?
3. **Threshold aktivasi**: default `eager` (aman) vs `static` sebagai default baru? Usul: `static` default hanya setelah 1 rilis dengan flag.
4. **Interaksi dengan `[tools] profile = "acp"/"minimal"`**: tiering harus komposabel dengan profile yang ada, bukan menggantikannya.

## 6. Rencana implementasi (setelah approval)

| # | Langkah | File utama | Test |
|---|---|---|---|
| 1 | Tambah `builtin_tiering`, `eager_tools`, `lazy_tools` ke `ToolConfig` + env override | `crates/jcode-base/src/config.rs` | `config_tests` |
| 2 | `tier_for_tool()` + daftar Tier 1/2 | `crates/jcode-app-core/src/tool/tier.rs` (baru) | unit test |
| 3 | Tandai `defer_loading` untuk Tier 2 di filter definitions | `turn_execution.rs` | `deferred_tools` test |
| 4 | Meta-tool `tools` (search/call) dispatch ke registry | `crates/jcode-app-core/src/tool/tools_meta.rs` (baru) | unit + integration |
| 5 | System prompt note 1 baris | `prompting.rs` / `build_system_prompt_split` | snapshot test |
| 6 | Log `Prompt prefix estimate` verifikasi turun ke < 11K | `~/.jcode/logs/` | manual check |

## 7. Acceptance criteria

- [ ] `builtin_tiering = "static"`: `Prompt prefix estimate` di log menunjukkan `tools < 4,000` dan `total < 11,000`.
- [ ] Workflow harian (read → edit → bash → todo) tidak memerlukan langkah tambahan.
- [ ] Tool Tier 2 (`browser`, `gmail`, dll) dapat dipanggil setelah 1 `tools` search/call.
- [ ] Tidak ada prompt-cache miss tambahan setelah turn pertama dalam sesi normal.
- [ ] `builtin_tiering = "eager"` (default) berperilaku identik dengan jcode saat ini — semua test lama lolos.
- [ ] Rollback = hapus 1 baris config.

## 8. Risiko

| Risiko | Mitigasi |
|---|---|
| Model "lupa" Tier 2 ada | Note 1 baris di system prompt + `tools` search selalu tersedia |
| Provider tanpa native deferred + tanpa transcript reference support | Fallback: definisi dimasukkan sebagai text di tool output (pola MCP existing) |
| Regression di model kecil yang tidak terbiasa dengan deferred flow | Tier 1 tetap mencakup 90% workflow coding; Tier 2 adalah tool niche |

## 9. Estimasi biaya

- Implementasi: ~1–2 hari (reuse MCP path, tanpa refactor registry).
- Risiko refactor: rendah — perubahan terisolasi di `turn_execution.rs` + 2 file baru + config.
