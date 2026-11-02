# Connect Server – återställd källkod 2026-11-02

Serverns kod återställdes från kompletta kodutskrifter och ändringar i projektets
bygglogg efter att den ursprungliga projektmappen raderats. Det här är en kopia
av serverprojektet, utan användardata, serveridentiteter eller privata nycklar.
Den bevarade klienten på skrivbordet är bygge ebea803d2b6bbd49.

Servern använder connect/server/1 och connect/control/1. Äldre lokala rum
använder connect/chat/8. Befintlig klient behöver inte bytas ut för återställningen.

Bygg från undermappen connect:

cargo build --locked --no-default-features --bin connect-server
cargo test --locked --no-default-features
cargo clippy --locked --no-default-features --all-targets -- -D warnings

Windowsbygget använder Rust 1.99.1 GNU och WinLibs GCC 16.2.0 UCRT POSIX SEH r3.
Kompilatorn och byggcache ingår inte i källkodsarkivet. vendor/iroh innehåller
1.4.1 med samma borttagning av cdylib som tidigare. Bibliotekets licens finns kvar.

Paketets README beskriver installation, engångsparkoppling, namnregistrering,
backup och återställning. Linux är fortfarande inte byggt/körtestat här.

Endast serverbygget ovan verifieras i denna återställning. Kvarvarande äldre
Rust-GUI-prototypfiler är referensmaterial; den separata Tauri-klientens källkod
är inte färdigverifierad eller inkluderad i denna serverbackup.
