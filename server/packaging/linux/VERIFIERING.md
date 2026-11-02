# Verifiering av Linux-paketet

Paketet är avsett för Intel/AMD x86_64 och innehåller en optimerad Linux-binär.

Körtestat i Alpine Linux 3.22.7 med Docker 28.3.3 och Caddy 2.11.7:

- Installation med medföljande install.sh och Docker Compose.
- HTTPS-panel, nekad felaktig nyckel och skyddad webbläsarsession.
- Skapa server, kanaler och inbjudningslänkar.
- Stoppa/starta med bibehållen serveridentitet och kanaler.
- Krypterad backup och återställning med samma serveridentitet.
- Ominstallation och återskapade containrar behåller serveridentitet,
  administrationsnyckel och kanaler.

Dessutom: 93 automatiserade Rust-tester godkända, 5 miljöberoende tester
överhoppade, formatkontroll och strikt Clippy godkända. Separat kodgranskning
utförd. Windows-programmet på skrivbordet har inte ersatts.

CasaOS-formatet har förberetts enligt dess dokumentation. Själva CasaOS-gränssnittet
och din fysiska server har inte körtestats här. Samtal och skärmdelning mellan
två användare över internet ingick inte i detta installationsprov.
