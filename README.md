# Claude Account & Session Studio

Claude Desktop'ta birden fazla Claude hesabı kullananlar için Windows uygulaması. Hesaplar arasında tek tıkla geçer, Code sekmesindeki sohbetleri her hesapta görünür tutar ve kotaları tek ekranda gösterir.

> **Uyarı:** Bu proje resmi değildir. Anthropic ile bir bağı yoktur, Anthropic tarafından onaylanmamış ya da desteklenmemektedir. Ayrıntılar aşağıdaki [Sorumluluk reddi](#sorumluluk-reddi) bölümünde.

## Ne yapar

- **Hesap geçişi:** Claude Desktop'u kapatır, açık hesabın giriş dosyalarını ve Claude Code CLI kimlik bilgilerini kaydeder, hedef hesabınkini geri yükler ve Claude Desktop'u yeniden açar. Tepsi menüsünden de geçilebilir.
- **Yeni hesap ekleme:** Mevcut hesabı kaydedip Claude Desktop'u temiz bir giriş ekranıyla açar; yeni giriş yapılan hesap otomatik tanınır ve kaydedilir.
- **Sohbet aktarımı:** Code sekmesi sohbet kayıtlarını (`local_*.json`) diğer hesaplara kopyalar. İki mod var: "geçişte sor" (yalnızca onayladığın sohbetler, gruplar ve klasörler aktarılır; geçişte eksik olanlar sorulur) ve "hepsi".
- **Kota takibi:** 5 saatlik ve haftalık kullanımı Claude Desktop'un yerel kaydından ya da Claude Code'un OAuth kullanım uç noktasından okur. Eşik aşılınca bir kez bildirim gösterir; hesabı kendiliğinden değiştirmez.
- **Tepsi:** Pencere kapanınca tepside çalışmaya devam eder; simge kotanın doluluğunu gösterir.
- **Yedekler:** Elle ya da otomatik SHA-256 doğrulamalı yedek alır; bir yedekten yalnızca eksik sohbetleri geri yükler.
- **Sağlık kontrolü:** Bozuk sohbet kayıtlarını ve doğrulaması tutmayan yedekleri salt okunur olarak listeler.
- **Otomatik güncelleme:** Yeni sürümü GitHub Releases'tan denetler, imzasını doğrular ve kurar; Ayarlar'dan kapatılabilir ya da elle denetlenebilir.
- Açık/koyu tema, Türkçe ve İngilizce arayüz (Windows diline göre ya da Ayarlar'dan seçilir), Ayarlar sayfası.

## Sohbetler nasıl güvende kalır

- **Yalnızca ekleme:** Aktarım eksik sohbeti kopyalar; hiçbir sohbet dosyasını silmez, taşımaz ya da üzerine yazmaz. Aynı sohbetin daha yeni bir kopyası varsa yalnızca tarih, tur sayısı ve başlık güncellenir; bu da yedekten sonra ve Claude Desktop kapalıyken yapılır.
- **Doğrulanmış yedek:** Yazmadan önce SHA-256 ile doğrulanmış bir yedek alınır. Yedekler Claude'un klasörlerinin dışında, `%LOCALAPPDATA%\ClaudeSessionSyncBackups` altında durur. Eski otomatik yedekler zamanla temizlenir, elle alınanlar silinmez.
- **Biçim koruması:** Claude Desktop'un dosya yapısı (örneğin bir güncellemeden sonra) tanınan biçime uymuyorsa tüm yazma işlemleri durur ve uyarı gösterilir.
- **Üzerine yazmayan geri yükleme:** Geri yükleme yalnızca eksik dosyaları ekler, var olanlara dokunmaz ve önce kendi yedeğini alır.
- Hesaba bağlı belirteçler (`bridgeSessionIds`) başka hesaba kopyalanırken temizlenir.

## Gereksinimler

- Windows 10 veya 11
- Claude Desktop (isteğe bağlı olarak Claude Code CLI)
- Derlemek için: Node.js, Rust ve [Tauri v2 önkoşulları](https://v2.tauri.app/start/prerequisites/)

## Derleme

```bash
npm install
```

```bash
npx tauri build
```

Kurulum dosyası `src-tauri/target/release/bundle/nsis/` altında oluşur.

## Örnek verilerle önizleme

```bash
npm run dev
```

Arayüz tarayıcıda Tauri olmadan, uydurma örnek verilerle (`src/devMock.ts`) açılır. Gerçek dosyalara dokunulmaz. `?uyari` biçim uyarısını, `?secim` geçişte sohbet seçme penceresini gösterir.

## Testler

```bash
cd src-tauri && cargo test
```

```bash
npx tsc --noEmit -p . && npm run build
```

## Sorumluluk reddi

- Bu proje **resmi değildir**; Anthropic ile bağlantılı değildir, Anthropic tarafından onaylanmamış ya da desteklenmemektedir. "Claude" Anthropic'in markasıdır.
- Claude Desktop'un **belgelenmemiş yerel dosyalarına** ve Claude Code'un **OAuth kullanım uç noktasına** dayanır. Bunlar her an haber verilmeden değişebilir ve uygulama çalışmaz hale gelebilir.
- Kimlik bilgileri bilgisayarında kalır ve Windows DPAPI ile şifrelenir (yalnızca senin Windows kullanıcın çözebilir).
- Uygulama Anthropic'in kendi uç noktaları (`api.anthropic.com`, `platform.claude.com`) dışında yalnızca güncelleme denetimi için GitHub'a (`github.com`) bağlanır; bu Ayarlar'dan kapatılabilir. Başka hiçbir yere veri göndermez; telemetri yoktur.
- **Kullanım riski sana aittir.** Yazılım "olduğu gibi" sunulur (bkz. [LICENSE](LICENSE)).

---

## English

A Windows app for people who use more than one Claude account in Claude Desktop. It switches between accounts in one click, keeps the Code-tab chats visible in every account and shows every account's quota on one screen.

> **Note:** This project is unofficial. It is not affiliated with, endorsed by or supported by Anthropic. See the [Disclaimer](#disclaimer) section below.

### What it does

- **Account switching:** closes Claude Desktop, saves the current account's sign-in files and Claude Code CLI credentials, restores the target account's, and reopens Claude Desktop. Also available from the tray menu.
- **Adding an account:** saves the current account and opens Claude Desktop on a clean sign-in screen; the newly signed-in account is recognized and saved automatically.
- **Chat copying:** copies Code-tab chat records (`local_*.json`) to the other accounts. Two modes: "ask when switching" (only the chats, groups and folders you approved are copied; missing ones are asked about at switch time) and "everything".
- **Quota tracking:** reads 5-hour and weekly usage from Claude Desktop's local record or from Claude Code's OAuth usage endpoint. Shows one notification when the threshold is crossed; it never switches accounts on its own.
- **Tray:** keeps running in the tray when the window is closed; the icon shows how full the quota is.
- **Backups:** manual or automatic backups verified with SHA-256; restoring from a backup only adds missing chats.
- **Health check:** lists broken chat records and backups that fail verification, read-only.
- **Auto-update:** checks GitHub Releases for a new version, verifies its signature and installs it; can be turned off or run by hand in Settings.
- Light/dark theme, Turkish and English interface (follows the Windows language or is picked in Settings), Settings page.

### How your chats stay safe

- **Additions only:** copying adds missing chats; no chat file is ever deleted, moved or overwritten. If a newer copy of the same chat exists, only its date, turn count and title are updated, and only after a backup and while Claude Desktop is closed.
- **Verified backup:** a SHA-256-verified backup is taken before any write. Backups live outside Claude's folders, under `%LOCALAPPDATA%\ClaudeSessionSyncBackups`. Old automatic backups are cleaned up over time; manual ones are never deleted.
- **Format guard:** if Claude Desktop's file layout no longer matches the known format (for example after an update), every write stops and a warning is shown.
- **Non-overwriting restore:** a restore only adds missing files, never touches existing ones, and takes its own backup first.
- Account-bound tokens (`bridgeSessionIds`) are cleared when a chat is copied to another account.

### Requirements

- Windows 10 or 11
- Claude Desktop (optionally the Claude Code CLI)
- To build: Node.js, Rust and the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/)

### Building

```bash
npm install
```

```bash
npx tauri build
```

The installer is created under `src-tauri/target/release/bundle/nsis/`.

### Preview with sample data

```bash
npm run dev
```

The interface opens in the browser without Tauri, with made-up sample data (`src/devMock.ts`). No real files are touched. `?uyari` shows the format warning, `?secim` shows the chat-picking dialog at switch time.

### Tests

```bash
cd src-tauri && cargo test
```

```bash
npx tsc --noEmit -p . && npm run build
```

### Disclaimer

- This project is **unofficial**; it is not affiliated with, endorsed by or supported by Anthropic. "Claude" is a trademark of Anthropic.
- It relies on Claude Desktop's **undocumented local files** and on Claude Code's **OAuth usage endpoint**. Both can change at any time without notice, and the app may stop working.
- Credentials stay on your computer, encrypted with Windows DPAPI (only your Windows user can decrypt them).
- Apart from Anthropic's own endpoints (`api.anthropic.com`, `platform.claude.com`), the app only connects to GitHub (`github.com`) to check for updates, which can be turned off in Settings. It sends data nowhere else; there is no telemetry.
- **Use at your own risk.** The software is provided "as is" (see [LICENSE](LICENSE)).
