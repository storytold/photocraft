# PhotoCraft 0.16.0 — bilanciamento del bianco e look colore

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
- I preset usano i coefficienti della fotocamera quando disponibili e supportati; altrimenti usano illuminanti CIE. Non usano il motore Adobe e non garantiscono una resa identica a Photoshop.
- I CR2 con Canon ColorData versione 14 (inclusa EOS 2000D) usano i coefficienti Canon incorporati per luce diurna, nuvolo, ombra, tungsteno, fluorescente e flash. Non è una calibrazione cromatica completa e i valori Canon possono differire dai preset Adobe.
- Negli altri RAW, quando manca la calibrazione della fotocamera, i preset di illuminazione restano approssimativi e il programma mostra un avviso.
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

Prima dei valori CIE vengono cercati i coefficienti RGGB incorporati nel CR2:
`camera_white_balance`, Canon ColorData8 versione 14, con conversione delle due
componenti verdi nella loro media. Layout da documentazione pubblica:
https://exiftool.org/TagNames/Canon.html#ColorData8
Le altre versioni, i dati troncati e i coefficienti non validi usano il fallback.
Automatico resta una stima dell'immagine, non il valore Auto memorizzato dalla fotocamera.

Nel fallback la calibrazione presente nel RAW viene usata tramite `ColorInfo::xy_to_neutral`.
In sua assenza viene dichiarato il modello approssimativo a primarie sRGB.
Automatico usa la stima grey-world già presente nel decoder.

Il controllo del dialogo accetta una richiesta autonoma:
`{"ui":{"whiteBalance":"cloudy"}}`, con ID `asShot`, `auto`, `daylight`, `cloudy`, `shade`, `tungsten`, `fluorescent`, `flash`.
Il valore corrente è esposto in `openingRaw.whiteBalance`.

## Look colore PhotoCraft

Sopra Luce, il menu **Profilo** offre sei look originali: **Standard, Colori,
Intenso, Paesaggio, Ritratto, Monocromatico**. I nomi includono PhotoCraft.
Standard conserva la resa precedente. Colori aggiunge contrasto e saturazione
leggeri; Intenso li aumenta; Paesaggio enfatizza blu e verdi; Ritratto usa un
contrasto più morbido e una saturazione contenuta; Monocromatico converte in
bianco e nero. Sono trasformazioni artistiche indipendenti, non calibrazioni
della fotocamera né copie o ricostruzioni esatte dei profili Adobe.

La scelta si combina con i cursori senza sovrascriverli, supporta annulla/ripeti,
resta selezionata cambiando bilanciamento del bianco ed è disponibile anche nel
filtro Camera Raw sulle immagini già aperte. Per ripartire dalla resa precedente
scegli Standard.

Il profilo viene salvato nei documenti `.pcraft` e nelle impostazioni XMP di
PhotoCraft tramite `pcraft:ColorProfile` nello spazio dei nomi
`urn:photocraft:camera-raw:1.0`. Photoshop ignora questa estensione. I risultati
applicati a un livello raster si salvano normalmente in PSD o nei formati immagine;
un filtro avanzato con un look PhotoCraft non ha una mappatura Adobe verificata
e l'esportazione PSD lo segnala. Per conservare l'editabilità usa `.pcraft`.

Il controllo del dialogo accetta `{"ui":{"set":{"profile":"landscape"}}}`.
Gli ID sono `standard`, `color`, `vivid`, `landscape`, `portrait`, `monochrome`.
Il comando `filter.cameraRaw` accetta direttamente lo stesso campo `profile`.
I PSD forniti sono riferimenti su una sola scena: non dimostrano una corrispondenza
generale con i profili Adobe, in particolare sui toni della pelle.
