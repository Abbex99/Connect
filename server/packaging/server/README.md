# Connect Server

Fristående server med egen identitet. Din Connect-profil är användare/ägare och
behöver inte vara öppen när servern körs. Servern kräver ingen OBS, mikrofon eller
grafisk skrivbordsmiljö. Datorn och servertjänsten måste vara igång för drift 24/7.

## Första starten på Windows

1. Packa upp serverpaketet i en egen mapp där du har skrivbehörighet.
2. Dubbelklicka på **Starta Server.bat**. Webbläsaren öppnar den lokala panelen.
3. Första gången: välj namn och adress i panelen och tryck **Skapa och starta server**.
4. Under **Inbjudningar** skapar du vanliga länkar till dina vänner. De klistrar in länken i Connect → Anslut.
5. Hantera kanaler, medlemmar, lagring och inbjudningar direkt i panelen. Start/stop finns ovanför den.
6. Nästa gång startas samma server automatiskt. Dubbelklick igen öppnar samma panel utan en extra server.

Ingen Connect-inloggning eller parkopplingskod behövs för lokal administration.
Behåll serverfönstret öppet. Du kan stänga webbläsarfliken; servern fortsätter köra.
Avsluta hela serverprogrammet med Ctrl+C i serverfönstret.

Datakatalogen `data` innehåller serverns hemliga identitet och historik. Den ska
inte skickas som vanlig ZIP. Serverkanaler kan läsas av serveroperatören.
Privata vänchattar ingår inte i denna server. Samtal och skärmdelning spelas inte in.
P2P-filer som användare skickar är inte automatiskt serverlagrade bilagor.

## Panel i webbläsare

Startfilen öppnar normalt `http://127.0.0.1:4749`. Om porten är upptagen väljs en ledig lokal port.
Använd startfilen för att öppna panelen: den ger åtkomst genom en lokal nyckel som skyddas av ditt Windows-konto.
Nyckeln ligger i `data.panel`; dela aldrig denna mapp. Webbläsarsessionen varar i högst tolv timmar.
Panelen lyssnar endast på den egna datorn och ska inte publiceras genom en proxy.

Den äldre fjärrpanelen finns kvar genom kommandot `run`, normalt på port 4748.
Den kräver fortfarande godkännande av en ägare/administratör genom Connect.
För fjärrpanel krävs HTTPS-proxy och exakt `panel_origin` i `data/server.json`.
`manage` och `run` får inte köras samtidigt mot samma data.

## Servernamn och internet

Inbjudningslänkar använder befintlig Iroh-transport och externt relä vid behov.
Besökare behöver inte skriva IP eller port. För namnanslutning behövs en gemensam
internetåtkomlig adressbok som kör denna version av `connect-directory`.
För en ny server kan du först välja **Drift och säkerhetskopiering → Skapa första ägarlänken** i lokalpanelen och ansluta med den i din egen Connect-app. Detta fungerar bara om servern ännu saknar ägare/administratör. Öppna sedan serverpanelen i Connect och välj **Registrera servernamnet**. Uppdateringen
publiceras inom en minut. Klientens adressboksinställning styr vilken adressbok
som används. Äldre adressböcker stöder inte den nya separata serveridentiteten.
Ingen publik adressbok eller hostingtjänst driftsätts av detta paket.

För eget relä kan servern startas med `--relay https://...`; klientens reläval
hanteras som tidigare i Connect. Relä och adressbok är separata tjänster.

## Historik och lagring

Meddelanden bekräftas först efter sparning. Historik visas i sidor om högst 100
meddelanden. Standardbudgeten för databasen är 1 GiB; konfigurera den i panelen.
När budgeten är slut avvisas nya skrivningar tydligt i stället för tyst radering.
Bevarandetid är avstängd från början; en vald tidsgräns verkställs regelbundet.
Profilmedier använder en separat begränsad cache. Serverns disk behöver också
utrymme för journaler och säkerhetskopior. Övervaka ledigt utrymme vid drift.

## Säkerhetskopiering och flytt

Öppna **Drift och säkerhetskopiering** i lokalpanelen. Skapa och spara en återställningsnyckel,
kontrollera att filen laddats ned och skapa sedan en backup. Backupen sparas i `data.backups`.
Spara den publika nyckeln (age1…) för kommande backuper. Förvara nyckelfilen separat från backupen.

För att återställa: packa upp programmet i en ny mapp, starta BAT-filen och välj
**Återställ en säkerhetskopia** i första-start-guiden. Ange sökvägar till backup och nyckelfil.
En befintlig datamapp skrivs aldrig över.

Alternativ för tjänstedrift (PowerShell):

```powershell
.\connect-server.exe recovery-key --out D:\SakerPlats\connect-recovery.txt
.\connect-server.exe backup --data .\data --out D:\Backup\connect.age --recipient age1...
.\connect-server.exe restore --backup D:\Backup\connect.age --data .\ny-data --key-file D:\SakerPlats\connect-recovery.txt
```

Den första kommandoraden visar en **publik mottagarnyckel** som används i nästa
kommando. Den hemliga återställningsnyckeln sparas i filen och visas inte i loggen.
Förvara den separat från backupfilen. Utan nyckeln kan backupen inte återställas.
Välj nya målfilnamn; befintliga backupfiler skrivs inte över.

Backup kan tas medan servern körs. Återställning sker till en ny datakatalog.
Stäng den gamla servern innan den återställda startas. Två aktiva kopior med samma
serveridentitet stöds inte. Serverbackup innehåller inte din privata användarprofil.
Browser-sessioner och oförbrukade ägarparkopplingar följer inte med återställningen.

## Flytta från ett äldre lokalt Connect-rum

Skapa en fristående server. Anslut först med dess ägarparkopplingslänk. Om samma
serveradress redan används av ditt gamla rum: lämna det gamla rummet och markera
**Flytta ett av mina egna äldre rum** i Anslut-dialogen. Namnfullmakten signeras
på din dator; din privata användarnyckel skickas inte till servern.

Under serverpanelen → Lagring anger du det gamla rummets `@namn/rum` och väljer
**Exportera signerad historik**. Stoppa nya servern och importera filen:

```powershell
.\connect-server.exe import-legacy --data .\data --file .\connect-server-migration.json
```

Målet måste sakna meddelanden och tidigare import. Originalhistoriken lämnas kvar.
Endast historik som fortfarande finns sparad i den gamla versionen kan följa med.
Medlemskap, roller och inbjudningar skapas på nytt; gamla författar-ID:n behålls.
Efter namnpubliceringen kan uppgraderade klienter verifiera flytten via adressboken.
Privata servrar kan behöva ge gamla gäster nya inbjudningar.

## Återställ administratörsåtkomst

Stoppa servern. Kör `Start-Server.ps1 -Pair` på servermaskinen. Detta återkallar
tidigare förhöjda roller och skapar en ny ägarparkoppling. Vanliga serveromstarter
ska göras utan `-Pair`. Skydda åtkomsten till datorn och serverns datakatalog.

## Linux och drift

Linux-paketet är källkod och en systemd-mall; Linux har inte körtestats i denna
Windows-miljö. Bygg med `cargo build --release --no-default-features --bin connect-server`.
Använd ett separat tjänstekonto, installera binären under `/opt/connect-server`,
och ge endast tjänstekontot åtkomst till `/var/lib/connect-server` (läge 0700).
Initiera/parkoppla som det kontot före installation av systemd-mallen. Anpassa
sökvägarna och starta tjänsten utan `--pair`. Använd ett övervakat system med
säkerhetskopior och tillräcklig bandbredd. En extern server garanterar inte i sig
bättre bildfrekvens eller lägre ljudfördröjning.

Windows-startfilen är en manuell startlösning, ingen installerad Windows-tjänst.
Automatiska uppdateringar, betald hosting och mobilappen kommer senare.
