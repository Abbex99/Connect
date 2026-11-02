# Connect Server – Linux / CasaOS (Intel och AMD)

Detta är den fristående servern, inte Connect-klientappen. Windows-appen på din
vanliga dator ansluter med inbjudningslänkar som tidigare. Servern kan köras 24/7.
Ingen Windows-miljö, OBS eller grafisk Linux-miljö behövs på serverdatorn.

## Installera

1. Kopiera och packa upp hela paketet på datorn med CasaOS.
2. Öppna en terminal där paketet ligger och kör `sudo sh install.sh`.
3. Skriv CasaOS-datorns lokala IPv4-adress, till exempel `192.168.1.50`.
4. Installationen skriver ut paneladressen och din privata administrationsnyckel.
5. Öppna paneladressen på din vanliga dator, klistra in nyckeln och skapa servern.

Docker och Docker Compose måste redan fungera. CasaOS-installationer använder
Docker, men vissa äldre installationer kan behöva Compose-pluginet.
Första installationen hämtar Caddys officiella Docker-image från internet.
Connect-binären ingår redan; du behöver inte installera Rust eller kompilera.
Installationen bygger en lokal image och publicerar inget på Docker Hub.

Start och stop fungerar från CasaOS eller med `sudo sh start.sh` / `sudo sh stop.sh`.
Servern startar automatiskt efter omstart om den inte uttryckligen stoppats.
CasaOS kan visa den befintliga Compose-appen. Om den inte visas kan du importera
`/DATA/AppData/connect-server/docker-compose.yml` via Custom Install.
Den lokala Connect-imagen måste först ha skapats genom `install.sh`.

## HTTPS och första besöket

Panelen använder ett lokalt HTTPS-certifikat från Caddy. Webbläsaren känner inte
automatiskt till denna lokala utfärdare. Kontrollera att adressen är din egen
CasaOS-dator. För varningsfri åtkomst kan du kopiera den publika CA-filen
`/DATA/AppData/connect-server/caddy-data/caddy/pki/authorities/local/root.crt`
från servern och importera den som betrodd rotutfärdare på den dator som ska
administrera servern. Det är ett aktivt förtroende för just din server.
Kopiera aldrig `root.key` eller andra privata certifikatnycklar.

Endast HTTPS-port 4750 publiceras. Den interna panelporten 4749 ska inte öppnas
separat. Detta paket är avsett för ditt lokala nätverk; vidarebefordra inte
administrationsporten i routern för att bjuda in vänner. Deras Connect-trafik
använder serverns vanliga transport och relä vid behov.

## Administration och data

I panelen skapar du servern, startar/stoppar, hanterar kanaler, medlemmar och
inbjudningar samt tar krypterade säkerhetskopior. Namnregistrering i adressboken
kräver fortfarande ägarens Connect-profil; vanliga inbjudningslänkar skapas direkt.

All beständig information ligger under `/DATA/AppData/connect-server`:

- `state/data`: serveridentitet, inställningar och historik.
- `state/operator.key`: den privata administrationsnyckeln (inte ditt användarkonto).
- `state/data.backups`: krypterade säkerhetskopior från panelen.
- `caddy-data`: HTTPS-certifikat och den lokala certifikatutfärdaren.

Radera inte dessa mappar när du byter programversion. Kör `install.sh` från det
nya paketet för uppdatering: befintlig data och administrationsnyckel återanvänds.
Uppdateringen startar om servern och kopplar från anslutna användare en kort stund.
Spara en backup innan uppdatering. Behåll gärna föregående paket för återgång.
Administrationssessioner upphör vid serverprogrammets omstart.
Återställningsnyckeln för backup laddas ned till datorn där webbläsaren är öppen;
förvara den separat från backupen.

Återställning i första-start-guiden använder sökvägar inuti containern. Lägg
backup och nyckelfil i `state/import` på serverdatorn, tillgängliga för UID 10001,
och ange `/state/import/filnamn` i panelen. Återställ till en ny, tom datamapp.
Stoppa den gamla servern först så att två kopior inte kör samma identitet.

## Felsökning

`sudo docker compose -f /DATA/AppData/connect-server/docker-compose.yml logs --tail 100`

Panelen accepterar exakt den adress som valdes vid installationen. Om IP-adressen
ändras: ändra `panel-host.txt` till den nya IPv4-adressen och kör `install.sh` igen.
En reserverad adress i routern minskar risken att adressen ändras.
Nyckeln kan visas lokalt med `sudo cat /DATA/AppData/connect-server/state/operator.key`.

## Bygga från källkod

Källkod levereras separat. Med Rust 1.99 eller senare och Linux C-byggverktyg:
`cargo build --locked --release --no-default-features --bin connect-server`.
Servern kan även köras utan Docker med en HTTPS-proxy framför `manage`.
Kör `./connect-server --help` för flaggorna.

Referenser: [CasaOS Compose-format](https://github.com/IceWhaleTech/CasaOS-AppStore/blob/main/docs/specs/compose-and-x-casaos.md),
[Caddys lokala TLS](https://caddyserver.com/docs/caddyfile/directives/tls),
[officiell Caddy-image](https://hub.docker.com/_/caddy).
