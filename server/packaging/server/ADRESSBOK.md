# Webblänkar för Connect Server

Starta via Starta Server.bat. Den nya servern registrerar sin aktuella adress automatiskt på https://mailhantering.se. Kontrollera att Översikt visar att serverlänken är online. Kopiera Serveradress överst i panelen, till exempel mailhantering.se/s/spelkvall. Välj själv länknamn och valfritt lösenord vid första starten.

Vännen öppnar adressen i webbläsaren, anger lösenord om det behövs, klickar Kopiera anslutning och klistrar in under Anslut i Connect. Länken ger åtkomst: dela inte ägarlänken med vänner. Normalinbjudningar har begränsad giltighet och antal användningar och går att återkalla i panelen.

Servern måste vara igång. Registreringen förnyas varje minut; adressen slutar lämnas ut efter tre minuter utan förnyelse. Ingen routerinställning behövs i normalfallet. Nätverk som blockerar relätrafik kan fortfarande förhindra anslutning; adressboken är ingen garanti mot nätverksblockeringar.

Uppdatera en befintlig server: stoppa den via panelen, stäng serverprocessen och byt programfilerna. Behåll data och data.panel. Serveridentiteten i data/identity.key gör att befintliga länkar fortsätter peka på samma server. Radera eller ersätt inte datamappen med en ny installation.

Egen adressbok: stoppa servern och ändra Adressbok under Drift och säkerhetskopiering, eller sätt CONNECT_DIRECTORY_URL. Tomt stänger av automatisk registrering. Serveridentiteten och din personliga Connect-profil är separata. Registrering av @namn/rum under en personlig identitet kan fortfarande göras med ägarprofilen i appen.

Adressboken sparar serverns publika identitet, namn och signerade nätverksadress, aldrig inbjudningsnyckeln. Meddelanden lagras på din server. Transporten använder externa Iroh-reläer som reserv; detta är separat från adressboken.

Lösenord: stoppa servern, öppna Drift och säkerhetskopiering och ange ett nytt lösenord. Tomt behåller det nuvarande. Markera Öppen server för att ta bort lösenordet. Sparade medlemsroller bevaras; ägare kan återställa servern. När ett länknamn redan är upptaget behöver du välja ett annat. Panelen visar inte en delningsadress innan registreringen är godkänd.
