# PhotoCraft 0.16.0 — prova del bilanciamento del bianco

Questa versione aggiunge, durante l'apertura di un RAW, il menu **Colore → Bilanciamento del bianco**:
Come scattato, Automatico, Luce diurna, Nuvolo, Ombreggiatura, Tungsteno, Fluorescente e Flash.

## Come provarla su Windows

1. Estrai completamente lo ZIP `PhotoCraft-Bilanciamento-Windows-x64`.
2. Apri `PhotoCraft.exe` dalla cartella estratta. La cartella `PhotoCraftData` conserva le impostazioni di questa copia portatile.
3. Apri un file RAW (ad esempio `.CR2`), espandi **Colore** e scegli il bilanciamento del bianco.
4. Aspetta che finisca lo sviluppo: l'immagine viene ricalcolata dal RAW originale. Le regolazioni dei cursori vengono mantenute.
5. Premi **Apri**, quindi salva o esporta l'immagine nel formato desiderato.

## Limiti della prima versione

- È una build personalizzata di prova, senza firma digitale del produttore.
- Sono preset di PhotoCraft, basati su illuminanti CIE. Non usano il motore Adobe e non garantiscono una resa identica a Photoshop.
- Quando manca la calibrazione della fotocamera, i preset di illuminazione sono approssimativi. Questo vale anche per i CR2 attualmente decodificati da PhotoCraft. Il programma mostra un avviso.
- La temperatura resta una regolazione relativa da −100 a +100; non è un valore in Kelvin.
- I profili Adobe Colori, Ritratto e Paesaggio non sono inclusi.
- Il menu richiede l'apertura del RAW originale: non appare nel filtro su un JPEG o su un livello già sviluppato.
- Ogni scelta ricostruisce la base di sviluppo: **Prima** confronta le regolazioni con quella base. Per ritrovare il bilanciamento originale scegli **Come scattato**.
- La cronologia dei cursori ricomincia dopo un cambio di preset. Il preset di sviluppo non viene incluso nel salvataggio XMP dei cursori.

## Compilazione su GitHub

Nel fork, apri **Actions**, abilita i workflow se GitHub lo richiede e seleziona
**PhotoCraft - bilanciamento del bianco Windows**. Per avviare una compilazione manuale,
il file del workflow deve esistere anche sul ramo predefinito: scegli poi **Run workflow**
e il ramo **camera-raw-white-balance**. In alternativa, una modifica a quel ramo avvia la compilazione automaticamente.

Il workflow esegue i test RAW, quelli di importazione e del dialogo, verifica il menu italiano,
Clippy, i livelli delle dipendenze e la compilazione WebAssembly, poi compila Windows x64.
Solo se tutti i passaggi passano pubblica lo ZIP negli **Artifacts** della singola esecuzione.
Non crea release pubbliche e non richiede certificati di firma.

## Dettagli per chi sviluppa

Gli illuminanti sono D55 (luce diurna e flash), D65 (nuvolo), D75 (ombra), A (tungsteno)
e F2 (fluorescente), con coordinate CIE 1931 a 2°. Coordinate di riferimento:
https://www.colour-science.org/api/0.3.1/html/_modules/colour/colorimetry/dataset/illuminants/chromaticity_coordinates.html

La calibrazione presente nel RAW viene usata tramite `ColorInfo::xy_to_neutral`.
In sua assenza viene dichiarato il modello approssimativo a primarie sRGB.
Automatico usa la stima grey-world già presente nel decoder.

Il controllo del dialogo accetta una richiesta autonoma:
`{"ui":{"whiteBalance":"cloudy"}}`, con ID `asShot`, `auto`, `daylight`, `cloudy`, `shade`, `tungsten`, `fluorescent`, `flash`.
Il valore corrente è esposto in `openingRaw.whiteBalance`.
