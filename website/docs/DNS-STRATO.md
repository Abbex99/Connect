# Koppla mailhantering.se till Connect

Hemsidan fungerar redan på https://connect-web-zeta.vercel.app . Domänerna mailhantering.se och www.mailhantering.se är tillagda och verifierade i Vercel-projektet connect-web.

Nästa steg i Stratos DNS-inställningar, enligt Vercels projektspecifika rekommendation den 2 november 2026:

| Typ | Namn | Värde |
| --- | --- | --- |
| A | @ (huvuddomänen) | 216.198.79.1 |
| A | @ (huvuddomänen) | 64.29.17.1 |
| CNAME | www | e35271a2939d3245.vercel-dns-017.com. |

Ersätt huvuddomänens tidigare webb-A-post (217.160.0.23) med dessa Vercel-adresser. För www ersätt nuvarande CNAME mailhantering.se. med Vercels värde. Låt e-postens MX- och TXT-poster vara kvar. Byt inte namnservrar. Kontrollera eventuella befintliga AAAA-poster innan omkopplingen; de får inte fortsätta leda webbtrafik till den gamla tjänsten.

DNS har ännu inte ändrats av oss eftersom Strato-åtkomst saknas. Det är först efter DNS-ändring och utfärdat HTTPS-certifikat som den egna domänen är klar. www skickas vidare till huvuddomänen med en tillfällig 307-omdirigering.

Detta kopplar hemsidan. Connects gemensamma adressbok och serverlänkar är ännu inte implementerade på den här webbplatsen.
