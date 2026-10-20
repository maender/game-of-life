# Game of Life — infini, rapide et joli

Un Game of Life (règles classiques B3/S23) en Rust, natif et plein écran, sur un **univers infini**. On zoome du gros plan jusqu'à des zones immenses, on randomise, on dessine à la souris, et on regarde la vie évoluer génération par génération.

## Lancer

Prérequis : Rust 1.91 ou plus récent. Testé sur macOS (Metal).

```bash
cargo run --release
```

L'application démarre en plein écran, en pause, avec une soupe aléatoire de la taille de l'écran. Appuie sur `Espace` pour lancer.

## Fonctionnalités

- **Univers infini** : pas de bord. Seules les zones vivantes occupent de la mémoire.
- **Zoom et déplacement** sur une très grande plage, du gros plan à un pixel pour des milliers de cellules (vue de densité).
- **Randomiser** la zone visible, avec une densité réglable. La zone est plafonnée à 268 M cellules pour ne pas geler la machine.
- **Dessiner et gommer** à la souris, en pause comme en marche.
- **Styles de cellule** : Bonbon (par défaut), Arrondis, Pastilles, Néon. Chaque cellule varie légèrement autour d'une couleur principale réglable.
- **Effets activables** :
  - **âge** : les cellules nées récemment sont claires, les anciennes plus sombres ;
  - **traînées** : une cellule morte s'efface progressivement ;
  - **naissance animée** : les cellules grossissent et rétrécissent en douceur.
- **Grille activable** : lignes fines gris clair sur les bords des cellules, qui s'effacent quand on dézoome.
- **Mode dégradé** : carrés pleins, sans effets, pour la vitesse maximale.
- **Panneau réductible** avec les statistiques : génération, population, images par seconde, générations par seconde, zoom.

## Commandes

| Touche | Action |
|---|---|
| `Espace` | Pause / play |
| `N` | Avancer d'une génération (en pause) |
| `+` / `−` | Vitesse |
| `R` | Randomiser la zone visible |
| `C` | Tout effacer |
| `D` / `E` | Outil dessiner / gommer |
| Clic gauche | Dessiner ou gommer |
| Molette / pincement | Zoom autour du curseur |
| Clic droit glissé / 2 doigts | Se déplacer |
| `H` | Recentrer sur la vie |
| `S` | Style suivant |
| `1` `2` `3` | Effets âge / traînées / animation |
| `G` | Grille on/off |
| `L` | Mode dégradé |
| `Tab` | Réduire / ouvrir le panneau |
| `F` | Plein écran on/off |
| `Échap` | Quitter |

Les raccourcis suivent le caractère tapé, donc ils marchent sur un clavier AZERTY. Les touches `1` `2` `3` marchent aussi sans Maj. Les combinaisons avec Cmd, Ctrl ou Alt sont ignorées : Cmd+C n'efface pas l'univers.

## Performances

Mesuré sur un MacBook M1 Pro (10 cœurs), en `--release`, sur une soupe de 4096×4096 cellules à 35 % (environ 5,9 M cellules vivantes) :

| Cas | Vitesse |
|---|---|
| Effets éteints | 435 à 466 générations/s |
| Effets âge + traînées | 90 à 117 générations/s |
| Vue de densité (calcul CPU, écran 3024×1964) | 2,6 à 3 ms par image |

Pour reproduire :

```bash
cargo run --release --example bench_soup
```

## Comment ça marche

- **Simulation** (`src/sim/`) :
  - l'univers est une table creuse de tuiles de 64×64 cellules, une ligne = un `u64` ;
  - chaque génération compte les voisins de 64 cellules à la fois avec des additionneurs bit à bit ;
  - les tuiles sont calculées en parallèle avec `rayon`, et seules les tuiles vivantes et leurs voisines sont calculées.
- **Thread de simulation** (`src/sim/runner.rs`) :
  - il reçoit des commandes (play, pause, dessin…) ;
  - il publie chaque génération comme un instantané partagé (`Arc`), sans copie et sans verrou côté rendu.
- **Rendu** (`src/render/`) :
  - `wgpu` dessine une quad par tuile visible, et le shader lit les bits pour dessiner le style de chaque cellule ;
  - les positions sont calculées par rapport à la caméra en entiers 64 bits, ce qui garde un rendu exact loin de l'origine ;
  - très dézoomé, une texture de densité calculée sur CPU remplace les tuiles.
- **Interface** (`src/ui/`, `src/app.rs`) : `winit` + `egui`.

La conception et le plan d'implémentation sont dans `docs/superpowers/`.

## Tests

```bash
cargo test
```
