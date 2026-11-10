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
- Açık/koyu tema, Türkçe arayüz, Ayarlar sayfası.

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

**Claude Account & Session Studio** is an unofficial Windows app (Tauri v2 + React) for people who use several Claude accounts in Claude Desktop.

- One-click account switching: closes Claude Desktop, saves the current account's login files and Claude Code CLI credentials, restores the target account's, and relaunches Desktop. Also available from the tray.
- Copies Code-tab chat pointers between accounts (additions only), either everything or only approved chats, with a prompt at switch time.
- Shows 5-hour and weekly quota from Claude Desktop's local record or Claude Code's OAuth usage endpoint, and notifies once near the limit. It never switches accounts on its own.
- Updates itself from GitHub Releases (signed); can be turned off in Settings.
- Verified SHA-256 backups, a restore that only adds missing files, a format guard that stops all writes if Desktop's file layout changes, and a read-only health check.

**Build:** `npm install`, then `npx tauri build`. **Dev preview with mock data:** `npm run dev`. Requires Windows 10/11 and Claude Desktop.

**Disclaimer:** Not affiliated with or endorsed by Anthropic. It relies on Claude Desktop's undocumented local files and on Claude Code's OAuth usage endpoint, both of which can change at any time. Credentials stay on your machine, encrypted with Windows DPAPI. Besides Anthropic's own endpoints it only contacts GitHub to check for updates, which can be turned off in Settings. Use at your own risk. MIT licensed.
