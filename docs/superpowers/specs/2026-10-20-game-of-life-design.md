# Game of Life — infini, rapide et joli

Date : 2026-10-20

## Intention

Un Game of Life (règles classiques B3/S23) très optimisé, lancé comme une application native en plein écran sur Mac. L'univers est **infini** : on peut zoomer et dézoomer librement, du gros plan sur quelques cellules jusqu'à une vue d'immenses zones. On regarde la vie évoluer **génération par génération** (vitesse réglable), on randomise, on met en pause, on dessine à la souris.

Le rendu doit être **joli** par défaut (cellules arrondies, en relief, couleurs qui varient légèrement autour d'une couleur principale), avec plusieurs styles et effets au choix, et un **mode dégradé** activable à la main qui sacrifie l'esthétique pour la vitesse.

Critère de réussite de l'utilisateur : toutes les fonctions ci-dessous marchent. Cible de performance (proposée, non contestée) : affichage fluide à ~60 FPS pendant zoom/déplacement même quand la simulation ralentit ; une soupe aléatoire de plusieurs millions de cellules vivantes avance encore à plusieurs dizaines de générations par seconde.

Machine cible : MacBook M1 Pro (10 cœurs, 32 Go), macOS. Rust 1.91 déjà installé.

## Objectifs

- Univers infini (pas de bord, pas de tore).
- Simulation pas à pas à vitesse réglable, pause / play, avancer d'une génération.
- Randomiser la zone visible, avec une densité réglable.
- Dessiner / gommer des cellules à la souris, y compris pendant que la simulation tourne.
- Zoom (molette / pincement, centré sur le curseur) et déplacement, sur une très grande plage.
- Styles de cellule, effets combinables, mode dégradé, panneau réductible, raccourcis clavier (détails plus bas).

## Non-objectifs

- Pas de HashLife ni de saut de millions de générations : on voit chaque génération. (Choix explicite : l'usage visé est de la grosse soupe aléatoire et du dessin, où HashLife est mauvais.)
- Pas d'autres règles que B3/S23 *(mon choix)*.
- Pas de sauvegarde / chargement de motifs (RLE, etc.), pas de bibliothèque de motifs *(mon choix — extension possible plus tard)*.
- Pas de version web ni multi-plateforme garantie (macOS d'abord ; si ça tourne ailleurs, tant mieux).

## Choix techniques décidés

Ces choix sont **les miens** (l'utilisateur m'a laissé choisir le langage), présentés et non contestés :

- **Langage : Rust.**
- **Rendu GPU via `wgpu`** (Metal sur Mac), fenêtre via `winit`. Les styles et effets sont faits dans les shaders pour que le joli rendu coûte peu au CPU.
- **Simulation en tuiles creuses** : l'univers est découpé en tuiles (ex. 64×64 cellules, une ligne = un `u64`). Seules les tuiles contenant de la vie (et leurs voisines potentiellement concernées) existent. Calcul bit-parallèle des voisins, tuiles réparties sur tous les cœurs.

## Comportement attendu

### Simulation
- Pause / play ; « 1 pas » avance d'exactement une génération (utile en pause).
- Vitesse réglable, jusqu'à « aussi vite que possible ».
- La simulation ne doit pas bloquer l'affichage : si elle est lente, l'image reste fluide et on continue de pouvoir zoomer/se déplacer/dessiner.

### Randomiser
- Remplit **la zone actuellement visible** à l'écran avec une densité réglable (défaut ~35 %). *Mon choix : la zone visible est remplacée (effacée puis remplie) ; le reste de l'univers n'est pas touché.*
- Si on est très dézoomé, la zone visible peut être immense : c'est accepté, avec la limite honnête que mémoire et vitesse dépendent du nombre de cellules vivantes. *Mon choix : plafonner la zone randomisée à une taille raisonnable (ordre de grandeur : quelques centaines de millions de cellules) et l'indiquer à l'utilisateur, plutôt que de geler la machine.*

### Dessin
- Outils « dessiner » et « gommer » (sélection au panneau ou `D` / `E`), clic gauche pour appliquer.
- Un trait rapide à la souris ne doit pas laisser de trous (interpoler entre positions successives).
- Fonctionne en pause comme en marche.

### Navigation
- Molette / pincement trackpad : zoom centré sous le curseur.
- Clic droit glissé / glissé à deux doigts : déplacement.
- `H` : recentrer la vue sur la vie (ex. sur l'ensemble des cellules vivantes).
- Plage de zoom : du gros plan (une cellule = plusieurs dizaines de pixels) à très loin (un pixel = beaucoup de cellules). Les coordonnées doivent rester exactes loin de l'origine (pas de dérive due aux flottants).

### Rendu selon le niveau de zoom
- Gros plan : cellules dessinées avec le style choisi.
- Quand une cellule ne fait plus que 1–2 pixels, les arrondis/halos/relief deviennent invisibles : on bascule naturellement sur des points simples (couleur et effets visibles conservés).
- Plus loin, un pixel représente plusieurs cellules : on affiche une **densité** (plus il y a de vivantes, plus c'est lumineux).
- Ce passage est automatique et distinct du **mode dégradé**, qui reste un interrupteur manuel.

## Apparence

Fond sombre *(mon choix, validé sur les maquettes)*.

### Styles de cellule (un seul actif, choisi au menu ou `S` pour passer au suivant)
- **C — Bonbon (défaut)** : carré arrondi avec petit espace entre cellules et dégradé vertical interne (plus clair en haut, plus sombre en bas) qui donne du relief.
- **A — Arrondis** : carré arrondi, couleur pleine, petit espace.
- **B — Pastilles** : cercles avec léger halo lumineux.
- **D — Néon** : seulement le contour arrondi, lumineux, intérieur presque transparent.

### Effets (chacun activable indépendamment, combinables avec n'importe quel style ; `1` `2` `3`)
- **E — Âge** : une cellule née récemment est claire, elle s'assombrit en vieillissant (plafonné à une dizaine de générations) ; les structures stables ressortent.
- **F — Traînées** : une cellule morte s'efface progressivement sur quelques générations au lieu de disparaître d'un coup.
- **G — Naissance animée** : entre deux générations, les cellules qui naissent grossissent et celles qui meurent rétrécissent en douceur.
- Ces effets demandent des données en plus par cellule (âge, trace) et peuvent ralentir un peu : ils sont désactivés par défaut *(mon choix)* et ne coûtent rien quand ils sont éteints.

### Couleur
- Une **couleur principale** choisie par l'utilisateur (défaut : turquoise `#3fb8a8`, celui des maquettes).
- Chaque cellule a une petite variation de teinte et de luminosité autour de cette couleur ; **l'intensité de variation est réglable**.
- La variation dépend de la position de la cellule (stable), pour éviter un scintillement permanent *(mon choix)*.

### Mode dégradé (`L`, manuel uniquement)
- Carrés pleins d'une seule couleur, sans espace, sans variation, effets ignorés.
- Doit apporter un gain réel de performance (rendu et, si effets actifs, simulation).

## Contrôles

### Panneau
Flottant en haut à gauche, semi-transparent, réductible (`Tab` ou bouton) en une petite barre qui ne garde que l'état play/pause et les compteurs. Sections :
- **Stats** : génération, population, FPS, générations/s, zoom.
- **Simulation** : pause/play, 1 pas, vitesse.
- **Grille** : randomiser la vue, densité, tout effacer.
- **Dessin** : dessiner / gommer.
- **Apparence** : style, couleur principale, variation, effets (âge / traînées / animation), mode dégradé.

Maquette validée : `.superpowers/brainstorm/*/content/panel.html`.

### Raccourcis
| Touche | Action |
|---|---|
| `Espace` | Pause / play |
| `N` | Avancer d'une génération |
| `+` / `−` | Vitesse |
| `R` | Randomiser la zone visible |
| `C` | Tout effacer |
| `D` / `E` | Outil dessiner / gommer |
| Molette / pincement | Zoom autour du curseur |
| Clic droit glissé / 2 doigts | Se déplacer |
| `H` | Recentrer sur la vie |
| `S` | Style suivant |
| `1` `2` `3` | Effets âge / traînées / animation |
| `L` | Mode dégradé |
| `Tab` | Réduire / ouvrir le panneau |
| `F` | Plein écran on/off |
| `Échap` | Quitter |

Au lancement : plein écran, simulation en pause, vue centrée sur l'origine, univers vide ou petite soupe aléatoire *(mon choix : une soupe aléatoire de la taille de l'écran, pour voir tout de suite quelque chose)*.

## Précisions (mes choix, après relecture)

- La cible de performance s'entend **effets désactivés**.
- En vue « densité » (très dézoomé), on n'affiche que la densité dans la couleur principale : styles, variation et effets sont ignorés (ils ne seraient de toute façon pas lisibles à cette échelle).
- Vitesse = nombre cible de générations par seconde, plus un cran « maximum ».
- « 1 pas » n'est actif qu'en pause.
- Raccourcis basés sur la touche logique (caractère produit), pour que `+` / `−` marchent sur un clavier AZERTY Mac.
- Une modification au dessin pendant qu'une génération est en calcul est appliquée à la génération suivante, jamais perdue.
- Coordonnées de cellules en `i64`.

## Laissé au constructeur

- Bibliothèque d'UI pour le panneau (ex. `egui` sur `wgpu`) et structure du code.
- Taille exacte des tuiles, structure de données des tuiles, stratégie de parallélisme, découplage simulation / rendu (threads, double tampon, etc.).
- Technique de rendu GPU par niveau de zoom (instances, textures de bits, pyramide de densité…), seuils de bascule.
- Valeurs par défaut fines (vitesse, intensité de variation, durée des traînées, rayon des coins).
- Tests : au minimum, vérifier la justesse de la simulation (motifs connus : clignotant, planeur, R-pentomino sur N générations, motifs traversant les bords de tuiles) et un banc de performance sur une grosse soupe.
