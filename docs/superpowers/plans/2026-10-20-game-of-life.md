# Game of Life infini — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Application native macOS plein écran : Game of Life B3/S23 sur univers infini, simulation bit-parallèle multi-cœur, rendu GPU joli (styles + effets), mode dégradé, panneau réductible et raccourcis.

**Architecture:** Bibliothèque `gol` (simulation pure, testable) + binaire. L'univers est une table creuse de tuiles 64×64 (`[u64; 64]`) ; chaque génération construit une nouvelle table en parallèle (rayon), publiée sans copie au rendu sous forme d'`Arc` via un thread de simulation piloté par commandes. Le rendu `wgpu` dessine une quad par tuile visible et le fragment shader lit les bits pour dessiner le style ; très dézoomé, une texture de densité calculée sur CPU remplace les tuiles. Panneau `egui`.

**Tech Stack:** Rust 1.91 (édition 2021), `winit`, `wgpu`, `egui` + `egui-wgpu` + `egui-winit`, `rayon`, `rustc-hash`, `arc-swap`, `bytemuck`, `pollster`. Versions : dernière version d'`egui-wgpu`, et les versions de `wgpu`/`winit` qu'elle requiert (une seule version de chacune dans `Cargo.lock`).

**Spec:** `docs/superpowers/specs/2026-10-20-game-of-life-design.md`

## Global Constraints

- Règles B3/S23 uniquement ; univers infini (ni bord ni tore).
- Coordonnées de cellules en `i64` ; `y` croît vers le bas ; cellule `(x, y)` → tuile `(x.div_euclid(64), y.div_euclid(64))`, local `(x.rem_euclid(64), y.rem_euclid(64))` ; dans une tuile, `rows[y]` bit `x` (bit 0 = colonne 0).
- Cible perf (effets éteints, `--release`, M1 Pro) : rendu ~60 FPS en zoom/déplacement indépendamment de la simulation ; soupe de plusieurs millions de vivantes à plusieurs dizaines de gén/s.
- Couleur principale par défaut `#3fb8a8` ; densité de randomisation par défaut 0,35 ; fond sombre `#0d1117`.
- Styles : C Bonbon (défaut), A Arrondis, B Pastilles, D Néon ; effets E Âge, F Traînées, G Animation, tous éteints par défaut ; mode dégradé manuel uniquement.
- « 1 pas » seulement en pause. Vitesse = gén/s cible ou « maximum ».
- Une édition (dessin, randomisation, effacement) n'est jamais perdue : appliquée avant la génération suivante.
- Raccourcis selon le caractère logique (AZERTY Mac).
- Profil `release` : `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`.

## Review Focus

1. **Trait de souris rapide, à travers les bords de tuiles et en coordonnées négatives** → trait continu sans trou. Test : `line_cells_is_8_connected` (Task 6) + `set_get_negative_and_across_tiles` (Task 2).
2. **Randomiser très dézoomé** → l'appli ne gèle pas ; la zone est plafonnée à `MAX_RANDOMIZE_CELLS` et centrée dans la vue. Test : `randomize_rect_is_capped` (Task 6).
3. **Touches `1` `2` `3` sur AZERTY** (le caractère produit sans Maj est `&` `é` `"`) → les effets basculent quand même. Test : `digit_keys_work_on_azerty` (Task 9), via le `KeyCode` physique `Digit1..3` en plus du caractère.
4. **Zoom extrême et univers vide** : molette poussée sans fin ou `H` sans cellule vivante → pas de NaN, pas de crash, zoom borné. Tests : `zoom_is_clamped`, `fit_empty_is_noop` (Task 6).
5. **La vie part au loin ou meurt** : un planeur qui voyage ou une population éteinte → les tuiles vides sont libérées (pas de fuite mémoire). Test : `glider_keeps_tile_count_bounded` (Task 3).

---

## File Structure

```
Cargo.toml
src/lib.rs                 — modules publics
src/main.rs                — lance app::run()
src/sim/mod.rs
src/sim/tile.rs            — Tile, TileCoord, conversions coordonnées
src/sim/step.rs            — step_tile bit-parallèle + référence naïve (tests)
src/sim/universe.rs        — Universe : table creuse, step parallèle, édition, randomisation
src/sim/effects.rs         — TileMeta (âge, traînée) et leur mise à jour
src/sim/runner.rs          — thread de simulation, SimCommand, Snapshot, Speed
src/view/camera.rs         — Camera (f64), écran↔cellule, zoom, fit, rectangle visible
src/view/line.rs           — rastérisation de trait
src/render/mod.rs          — Renderer (surface, pipelines, upload)
src/render/tiles.wgsl      — chemin « cellules » : styles + effets
src/render/density.wgsl    — chemin « densité »
src/render/density.rs      — calcul CPU de la texture de densité
src/ui/settings.rs         — Settings (style, couleur, effets, dégradé…)
src/ui/panel.rs            — panneau egui
src/ui/keys.rs             — Action, mapping clavier
src/app.rs                 — ApplicationHandler winit, câblage
tests/shaders.rs           — validation naga des WGSL
examples/bench_soup.rs     — banc de performance
```

---

### Task 1: Projet + Tile

**Files:**
- Create: `Cargo.toml`, `.gitignore` (`/target`, `.superpowers/`), `src/lib.rs`, `src/main.rs` (affiche un message, sera remplacé Task 9), `src/sim/mod.rs`, `src/sim/tile.rs`

**Interfaces:**
- Produces:
  - `pub const TILE: i64 = 64;`
  - `pub type TileCoord = (i64, i64);`
  - `pub fn split(x: i64, y: i64) -> (TileCoord, usize, usize)` (tuile, lx, ly)
  - `#[derive(Clone, Copy, PartialEq, Eq, Default)] pub struct Tile { pub rows: [u64; 64] }`
  - `impl Tile { fn get(&self, lx: usize, ly: usize) -> bool; fn set(&mut self, lx: usize, ly: usize, alive: bool); fn is_empty(&self) -> bool; fn population(&self) -> u32; fn border(&self) -> Border }`
  - `pub struct Border { pub n: bool, pub s: bool, pub w: bool, pub e: bool }` — vrai si la ligne 0 / ligne 63 / colonne 0 / colonne 63 contient une vivante.

- [ ] **Step 1:** `git init` à la racine, `cargo init --lib --name gol`, ajouter `[[bin]] name = "gol"` (src/main.rs), le profil release des Global Constraints, et les dépendances `rayon`, `rustc-hash`, `arc-swap`, `bytemuck` (feature `derive`). Les dépendances graphiques arrivent Task 7.
- [ ] **Step 2: Tests qui échouent** dans `src/sim/tile.rs` :
  - `split_handles_negatives` : `split(-1, -1) == ((-1, -1), 63, 63)`, `split(64, 0) == ((1, 0), 0, 0)`, `split(63, 127) == ((0, 1), 63, 63)`.
  - `set_get_population` : poser (0,0), (63,0), (5,63) → `population() == 3`, `get(63,0)`, `!get(1,0)`; effacer (0,0) → 2.
  - `border_flags` : tuile avec seule (0,10) → `w && !e && !n && !s` ; avec seule (63,63) → `e && s`.
- [ ] **Step 3:** `cargo test sim::tile` → FAIL (non défini).
- [ ] **Step 4:** Implémenter.
- [ ] **Step 5:** `cargo test sim::tile` → PASS.
- [ ] **Step 6:** Commit `feat: scaffold crate and Tile`.

---

### Task 2: Pas bit-parallèle d'une tuile

**Files:**
- Create: `src/sim/step.rs`

**Interfaces:**
- Consumes: `Tile` (Task 1).
- Produces:
  - `pub struct Neighborhood<'a> { pub c: &'a Tile, pub n: Option<&'a Tile>, pub s: Option<&'a Tile>, pub w: Option<&'a Tile>, pub e: Option<&'a Tile>, pub nw: Option<&'a Tile>, pub ne: Option<&'a Tile>, pub sw: Option<&'a Tile>, pub se: Option<&'a Tile> }` (`None` = vide). `n` = tuile `(tx, ty-1)`.
  - `pub fn step_tile(nb: &Neighborhood) -> Tile`
  - `#[cfg(test)] pub fn step_naive(cells: &HashSet<(i64,i64)>) -> HashSet<(i64,i64)>` — référence.

Algorithme : pour chaque ligne `y`, prendre les lignes `y-1`, `y`, `y+1` (venant de `n`/`s` aux bords). Pour chaque ligne `r` avec ses lignes voisines `w_r`, `e_r` des tuiles ouest/est (ou NW/NE/SW/SE pour les lignes hors tuile), voisin ouest de la colonne x = `(r << 1) | (w_r >> 63)`, voisin est = `(r >> 1) | (e_r << 63)`. On obtient 8 masques de voisins (3 de la ligne du dessus, 2 de la ligne courante, 3 du dessous) ; les additionner en compteurs binaires bit-parallèles (demi/plein additionneurs : bits s0, s1, s2 ; ≥4 saturé), puis `next = (count == 3) | (alive & count == 2)`.

- [ ] **Step 1: Tests qui échouent :**
  - `blinker_in_tile` : (10,10),(11,10),(12,10) → (11,9),(11,10),(11,11).
  - `blinker_across_east_border` : clignotant horizontal en (62,5),(63,5) dans `c` et (0,5) dans `e` → pas de `c` contient (63,4),(63,5),(63,6) uniquement.
  - `matches_naive_random` : 200 tirages pseudo-aléatoires (xorshift à graine fixe) de 9 tuiles à densité 0,4 ; `step_tile` sur la tuile centrale == cellules de `step_naive` restreintes à la tuile centrale.
- [ ] **Step 2:** `cargo test sim::step` → FAIL.
- [ ] **Step 3:** Implémenter `step_tile` et `step_naive`.
- [ ] **Step 4:** `cargo test sim::step` → PASS.
- [ ] **Step 5:** Commit `feat: bit-parallel tile step`.

---

### Task 3: Universe

**Files:**
- Create: `src/sim/universe.rs`

**Interfaces:**
- Consumes: `Tile`, `split`, `step_tile`, `Neighborhood`.
- Produces:
  - `pub type TileMap = rustc_hash::FxHashMap<TileCoord, Tile>;`
  - `#[derive(Clone, Copy, Debug, PartialEq)] pub struct CellRect { pub x0: i64, pub y0: i64, pub x1: i64, pub y1: i64 }` (x1, y1 exclusifs) avec `fn area(&self) -> u128`.
  - `pub struct Universe { tiles: Arc<TileMap>, generation: u64, population: u64 }`
  - `impl Universe { fn new() -> Self; fn get(&self, x: i64, y: i64) -> bool; fn set(&mut self, x: i64, y: i64, alive: bool); fn step(&mut self) -> Arc<TileMap> /* renvoie l'ancienne table */; fn tiles(&self) -> &Arc<TileMap>; fn generation(&self) -> u64; fn population(&self) -> u64; fn tile_count(&self) -> usize; fn clear(&mut self); fn fill_random(&mut self, rect: CellRect, density: f32, seed: u64); fn bounding_box(&self) -> Option<CellRect> }`
  - `pub fn bounding_box(tiles: &TileMap) -> Option<CellRect>` (fonction libre, utilisée aussi par l'app sur un snapshot).

Règles : `set` met à jour `population` (±1) et passe par `Arc::make_mut` (copie seulement si le rendu tient l'ancienne table) et supprime une tuile devenue vide. `step` : candidats = tuiles existantes + voisines du côté (et coin) où `border()` signale une vivante ; calcul en `rayon` `par_iter`, tuiles vides éliminées ; population = somme des `population()`. `fill_random` **remplace** le contenu de `rect` (efface puis remplit), en parallèle par tuile, générateur déterministe par tuile `(seed, tx, ty)` ; densité par masque binaire (combinaison de 8 mots aléatoires selon les bits de `round(density·256)`). `bounding_box` : précision à la tuile suffit.

- [ ] **Step 1: Tests qui échouent :**
  - `set_get_negative_and_across_tiles` : poser (-1,-1), (-64,0), (63,64), (i64::MAX/2, -5) → toutes lues vivantes, `population()==4` (maintenue par `set`) ; effacer (-1,-1) → `get` faux et tuile (-1,-1) absente (`tile_count()==3`).
  - `glider_crosses_tiles` : planeur standard en (60,60) ; après 4·k générations il est translaté de (k,k) ; vérifier k=10 (traverse les bords) et population 5.
  - `r_pentomino_1103` : R-pentomino → à la génération 1103, `population() == 116`.
  - `matches_naive_negative_region` : soupe 120×120 densité 0,35 centrée sur (0,0) (donc négatifs), 60 générations, ensemble des vivantes identique à `step_naive` (Task 2).
  - `glider_keeps_tile_count_bounded` : planeur seul, 1000 générations → `tile_count() <= 4` à chaque génération.
  - `fill_random_replaces_and_respects_density` : poser (1000,1000) hors rect ; `fill_random(CellRect{0,0,512,512}, 0.35, 7)` → population dans [0,33·512², 0,37·512²], (1000,1000) toujours vivante ; deuxième appel densité 0 sur même rect → population 1.
  - `bounding_box_empty_and_some` : vide → `None` ; cellule (-70, 5) → boîte contenant (-70,5).
- [ ] **Step 2:** `cargo test sim::universe` → FAIL.
- [ ] **Step 3:** Implémenter.
- [ ] **Step 4:** `cargo test sim::universe` → PASS (et `cargo test --release` pour le R-pentomino si lent).
- [ ] **Step 5:** Commit `feat: sparse parallel universe`.

---

### Task 4: Données d'effets (âge, traînées)

**Files:**
- Create: `src/sim/effects.rs`
- Modify: `src/sim/universe.rs` (champ et appel dans `step`)

**Interfaces:**
- Consumes: `TileMap`, `Tile`.
- Produces:
  - `pub const AGE_MAX: u8 = 12; pub const TRAIL_LEN: u8 = 4;`
  - `#[derive(Clone)] pub struct TileMeta { pub age: [u8; 4096], pub trail: [u8; 4096] }` (index `ly*64+lx`).
  - `pub type MetaMap = FxHashMap<TileCoord, TileMeta>;`
  - `pub fn update_meta(old: &MetaMap, new_tiles: &TileMap) -> MetaMap` — vivante : `age = min(age+1, AGE_MAX)`, `trail = TRAIL_LEN` ; morte : `age = 0`, `trail = trail.saturating_sub(1)`. Une tuile de méta n'existe que si elle a une vivante ou une traînée > 0. Parallèle (rayon).
  - Dans `Universe` : `fn set_effects(&mut self, on: bool)` (crée la méta à partir des tuiles actuelles avec âge 1, ou la supprime) ; `fn meta(&self) -> Option<&Arc<MetaMap>>` ; `step` appelle `update_meta` seulement si activé.

- [ ] **Step 1: Tests qui échouent :**
  - `block_ages_to_cap` : bloc 2×2, effets on, 20 générations → âge de chaque cellule == `AGE_MAX`.
  - `dead_cell_trail_fades` : une cellule isolée, effets on ; après 1 gén elle est morte, `trail == TRAIL_LEN-1` ; après `TRAIL_LEN` gén, la tuile de méta a disparu.
  - `effects_off_has_no_meta` : `meta()` est `None` par défaut et après `set_effects(false)`.
  - `newborn_age_is_one` : clignotant, cellule nouvellement née → âge 1.
- [ ] **Step 2:** `cargo test sim::effects` → FAIL.
- [ ] **Step 3:** Implémenter.
- [ ] **Step 4:** `cargo test` → PASS (toute la suite).
- [ ] **Step 5:** Commit `feat: age and trail effect data`.

---

### Task 5: Thread de simulation

**Files:**
- Create: `src/sim/runner.rs`

**Interfaces:**
- Consumes: `Universe`, `CellRect`, `TileMap`, `MetaMap`.
- Produces:
  - `pub const SPEEDS: [Speed; 10] = [Gps(1), Gps(2), Gps(5), Gps(10), Gps(20), Gps(30), Gps(60), Gps(120), Gps(240), Max];` défaut `Gps(30)`.
  - `#[derive(Clone, Copy, PartialEq, Debug)] pub enum Speed { Gps(u32), Max }` ; `fn faster(self) -> Speed`, `fn slower(self) -> Speed` (bornés dans `SPEEDS`).
  - `pub enum SimCommand { Play, Pause, Step, SetSpeed(Speed), Edit(Vec<(i64, i64, bool)>), Randomize { rect: CellRect, density: f32 }, Clear, SetEffects(bool), Shutdown }`
  - `pub struct Snapshot { pub tiles: Arc<TileMap>, pub prev: Arc<TileMap>, pub meta: Option<Arc<MetaMap>>, pub generation: u64, pub population: u64, pub running: bool, pub gens_per_sec: f32, pub published_at: Instant }`
  - `pub struct SimHandle { pub fn send(&self, c: SimCommand); pub fn snapshot(&self) -> Arc<Snapshot> }` ; `pub fn spawn(universe: Universe) -> SimHandle` ; `Drop` envoie `Shutdown` et joint le thread.

Boucle : en pause, `recv()` bloquant ; en marche, vider les commandes (`try_recv`), faire un `step`, publier (`ArcSwap`), dormir jusqu'à l'échéance `1/gps` (pas de sommeil en `Max`). Toute commande d'édition s'applique puis republie immédiatement (même en pause). `Step` ignoré en marche. `gens_per_sec` = moyenne glissante sur ~1 s. Graine de `Randomize` : compteur interne.

- [ ] **Step 1: Tests qui échouent** (utiliser une petite attente avec délai max 2 s sur la génération publiée) :
  - `step_in_pause_advances_one` : `spawn`, `Step` ×3 → génération 3, `running == false`.
  - `step_ignored_while_running` : `SetSpeed(Gps(1))`, `Play`, attendre 0,1 s, envoyer `Step` ×5, `Pause` → génération ≤ 2.
  - `edits_while_running_are_kept` : `SetSpeed(Max)`, `Play`, envoyer `Edit` d'un bloc 2×2 loin de tout, attendre 0,2 s, `Pause` → les 4 cellules sont vivantes dans `snapshot().tiles`.
  - `speed_bounds` : `Speed::Max.faster() == Max`, `Gps(1).slower() == Gps(1)`, `Gps(30).faster() == Gps(60)`.
  - `prev_is_previous_generation` : clignotant, `Step` → `prev` contient l'état horizontal, `tiles` le vertical.
- [ ] **Step 2:** `cargo test sim::runner` → FAIL.
- [ ] **Step 3:** Implémenter.
- [ ] **Step 4:** `cargo test sim::runner` → PASS.
- [ ] **Step 5:** Commit `feat: simulation thread with commands and snapshots`.

---

### Task 6: Caméra, trait, randomisation de la vue

**Files:**
- Create: `src/view/mod.rs`, `src/view/camera.rs`, `src/view/line.rs`

**Interfaces:**
- Consumes: `CellRect`.
- Produces:
  - `pub const MIN_PX_PER_CELL: f64 = 1.0 / 4096.0; pub const MAX_PX_PER_CELL: f64 = 128.0; pub const MAX_RANDOMIZE_CELLS: u128 = 1 << 28;`
  - `#[derive(Clone, Copy, Debug)] pub struct Camera { pub center: (f64, f64) /* en cellules */, pub px_per_cell: f64, pub screen: (f64, f64) /* pixels physiques */ }`
  - `impl Camera { fn screen_to_cell(&self, sx: f64, sy: f64) -> (f64, f64); fn cell_at(&self, sx: f64, sy: f64) -> (i64, i64) /* floor */; fn zoom_at(&mut self, sx: f64, sy: f64, factor: f64); fn pan_px(&mut self, dx: f64, dy: f64); fn visible_rect(&self) -> CellRect; fn fit(&mut self, r: Option<CellRect>) }`
  - `pub fn randomize_rect(cam: &Camera) -> (CellRect, bool /* plafonné */)` — rect visible, ou un carré centré sur la vue d'aire ≤ `MAX_RANDOMIZE_CELLS` (en gardant les proportions de l'écran).
  - `pub fn line_cells(a: (i64, i64), b: (i64, i64)) -> Vec<(i64, i64)>` (Bresenham, extrémités incluses).

`fit` : centrer sur la boîte et choisir `px_per_cell` pour qu'elle occupe ~80 % de l'écran, borné ; `None` → ne rien changer.

- [ ] **Step 1: Tests qui échouent :**
  - `zoom_keeps_point_under_cursor` : caméra quelconque, `p = screen_to_cell(300, 200)`, `zoom_at(300, 200, 1.7)` → `screen_to_cell(300,200)` à 1e-9 près de `p`.
  - `zoom_is_clamped` : `zoom_at(..., 1e9)` répété → `px_per_cell == MAX_PX_PER_CELL`, `zoom_at(..., 1e-9)` répété → `MIN_PX_PER_CELL`, jamais NaN.
  - `cell_at_floors_negatives` : centre (0,0), px_per_cell 10, écran 100×100 → `cell_at(49.0, 49.0) == (-1, -1)`, `cell_at(50.0, 50.0) == (0, 0)`.
  - `fit_empty_is_noop` : `fit(None)` ne change rien.
  - `randomize_rect_is_capped` : `px_per_cell = MIN_PX_PER_CELL`, écran 3456×2234 → `area() <= MAX_RANDOMIZE_CELLS` et drapeau vrai ; à `px_per_cell = 8` → rect == `visible_rect()`, drapeau faux.
  - `line_cells_is_8_connected` : de (-5,-3) à (37,-80) → premier/dernier corrects, chaque paire consécutive à distance de Chebyshev 1.
- [ ] **Step 2:** `cargo test view` → FAIL.
- [ ] **Step 3:** Implémenter.
- [ ] **Step 4:** `cargo test view` → PASS.
- [ ] **Step 5:** Commit `feat: camera, stroke rasterizer, randomize rect`.

---

### Task 7: Fenêtre et rendu des cellules (styles, couleur, dégradé)

**Files:**
- Modify: `Cargo.toml` (ajouter `winit`, `wgpu`, `pollster`, `egui`, `egui-wgpu`, `egui-winit`)
- Create: `src/ui/settings.rs`, `src/render/mod.rs`, `src/render/tiles.wgsl`, `tests/shaders.rs`
- Modify: `src/main.rs` (fenêtre plein écran minimale qui affiche un `Universe` statique pour vérifier — le câblage final est Task 9)

**Interfaces:**
- Consumes: `Camera`, `Snapshot`.
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Debug)] pub enum CellStyle { Candy, Rounded, Dots, Neon }` ; `const ALL: [CellStyle; 4] = [Candy, Rounded, Dots, Neon]` ; `fn next(self) -> Self` ; libellés `"C — Bonbon"`, `"A — Arrondis"`, `"B — Pastilles"`, `"D — Néon"`.
  - `pub struct Settings { pub style: CellStyle, pub color: [f32; 3] /* sRGB 0..1, défaut #3fb8a8 */, pub variation: f32 /* 0..1, défaut 0.35 */, pub effect_age: bool, pub effect_trails: bool, pub effect_anim: bool, pub degraded: bool, pub density: f32 /* 0.35 */, pub panel_open: bool /* true */, pub tool_erase: bool /* false */ }` + `Default` ; `fn effects_needed(&self) -> bool` = `!degraded && (effect_age || effect_trails)`.
  - `pub struct Renderer { pub fn new(window: Arc<Window>) -> Self; pub fn resize(&mut self, w: u32, h: u32); pub fn render(&mut self, cam: &Camera, snap: &Snapshot, s: &Settings, anim_t: f32, ui: Option<(&egui::Context, egui::FullOutput)>) }`

Rendu « cellules » (utilisé si `px_per_cell >= 0.5`) : uploader les tuiles visibles (rect visible + 1 tuile de marge) dans un storage buffer `struct GpuTile { rel: vec2<i32> /* tuile - tuile du centre caméra */, abs_lo: vec2<i32> /* 32 bits bas de la coordonnée absolue, pour le hash couleur */, rows: array<u32, 128> }`. Ne ré-uploader que si le `Snapshot` (comparer `Arc::ptr_eq`) ou la caméra a changé. Une quad instanciée par tuile ; uniforms : décalage caméra en cellules relatif à la tuile centrale (f32), `px_per_cell`, taille écran, couleur, variation, style, flags. Fragment : cellule locale → bit ; morte → `discard`.

Styles dans le shader (cellule en coordonnées locales `uv ∈ [0,1]²`, marge 1,5/16 de cellule, rayon 4/16) : Bonbon = rectangle arrondi (SDF) + dégradé vertical (+18 % luminosité en haut, −12 % en bas) ; Arrondis = rectangle arrondi plein ; Pastilles = disque rayon 0,5−2/16 + halo radial dans la cellule ; Néon = contour de largeur ~1,6/16 lumineux + intérieur alpha 0,15. Couleur : HSL de la couleur principale, teinte ±14°·(variation/0,35) et luminosité ±7 %·(variation/0,35) d'après un hash entier stable de la cellule absolue. Si `px_per_cell < 3` : carré plein (la forme est invisible). Si `px_per_cell < 1` : densité de l'empreinte du pixel (popcount du bloc couvert) → luminosité. Mode dégradé : carré plein sans marge, couleur principale exacte, sans variation ni effet. Antialiasing des SDF via `fwidth`.

- [ ] **Step 1: Test qui échoue** `tests/shaders.rs::tiles_wgsl_is_valid` : parse `include_str!("../src/render/tiles.wgsl")` avec `wgpu::naga::front::wgsl::parse_str` puis `naga::valid::Validator::new(ValidationFlags::all(), Capabilities::all()).validate(&module)` → `Ok`. Et test unitaire `style_cycle` : `Candy.next()==Rounded`, …, `Neon.next()==Candy`.
- [ ] **Step 2:** `cargo test --test shaders` → FAIL (fichier absent).
- [ ] **Step 3:** Implémenter `Settings`, `Renderer`, `tiles.wgsl`, et le `main.rs` provisoire (plein écran, soupe 512×512, caméra centrée, `Échap` quitte, molette zoom, chiffres temporaires pour changer de style).
- [ ] **Step 4:** `cargo test` → PASS ; `cargo run --release` : vérifier à l'œil chaque style à fort zoom, le passage aux carrés pleins puis aux points en dézoomant, le dégradé. Pas de saut ni de tremblement de la grille en déplaçant la vue loin de l'origine (tester `center = (1e12, 1e12)`).
- [ ] **Step 5:** Commit `feat: GPU cell rendering with styles`.

---

### Task 8: Effets au rendu et vue densité

**Files:**
- Modify: `src/render/mod.rs`, `src/render/tiles.wgsl`
- Create: `src/render/density.rs`, `src/render/density.wgsl`
- Modify: `tests/shaders.rs`

**Interfaces:**
- Consumes: `Snapshot::{prev, meta}`, `TileMeta`, `AGE_MAX`, `TRAIL_LEN`, `Settings`.
- Produces:
  - `pub fn density_grid(tiles: &TileMap, cam: &Camera, out_w: u32, out_h: u32) -> Vec<f32>` — fraction de vivantes par pixel de sortie (0..1), en parallèle (rayon) ; pour une tuile plus grande qu'un pixel, popcount du sous-bloc couvert ; sinon on ajoute la population de la tuile au pixel qui la contient.

Effets dans `tiles.wgsl` (ignorés en dégradé) : buffers parallèles à `GpuTile`, uploadés seulement si l'effet est actif : `age`/`trail` empaquetés 4×u8 par u32 (1024 u32 par tuile) et `prev_rows` (128 u32). Les tuiles uploadées = union des tuiles vivantes visibles et des tuiles de méta visibles (traînées sur tuiles mortes). Âge : luminosité de +25 % (né) à −10 % (âge `AGE_MAX`). Traînées : cellule morte avec trail>0 dessinée avec alpha `0,7·trail/TRAIL_LEN`. Animation : `anim_t ∈ [0,1]` ; née (`!prev && cur`) taille ×`anim_t`, morte (`prev && !cur`) taille ×`1−anim_t`, centrée. `anim_t` (calculé dans app, Task 9) = `min(1, (now − published_at) / (0,6 · période))`, `1` en `Max` ou en pause.

Vue densité si `px_per_cell < 0.5` : `density_grid` à la résolution de l'écran (divisée par 2 si nécessaire pour tenir 60 FPS), recalculée seulement si snapshot ou caméra changent, uploadée en texture `R32Float`, quad plein écran ; couleur principale × `sqrt(densité)` ; ni variation ni effet dans cette vue.

- [ ] **Step 1: Tests qui échouent :**
  - `density_full_tile` : une tuile pleine (tous bits à 1) en (0,0), caméra `px_per_cell = 1/64` centrée sur (32,32), sortie 3×3 → le pixel central vaut 1,0 (±1e-6) et les autres 0.
  - `density_half` : tuile avec 2048 vivantes, `px_per_cell = 1/64` → pixel central 0,5.
  - `density_wgsl_is_valid` dans `tests/shaders.rs` (même méthode que Task 7).
- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Implémenter.
- [ ] **Step 4:** `cargo test` → PASS ; `cargo run --release` : à l'œil, âge (blocs stables sombres, naissances claires), traînées, animation, passage fluide cellules → densité en dézoomant.
- [ ] **Step 5:** Commit `feat: effect rendering and density view`.

---

### Task 9: Application complète (panneau, raccourcis, dessin, navigation)

**Files:**
- Create: `src/ui/mod.rs`, `src/ui/keys.rs`, `src/ui/panel.rs`, `src/app.rs`
- Modify: `src/main.rs` → `fn main() { gol::app::run() }`

**Interfaces:**
- Consumes: tout ce qui précède.
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Debug)] pub enum Action { TogglePlay, Step, Faster, Slower, Randomize, Clear, ToolDraw, ToolErase, Recenter, NextStyle, ToggleAge, ToggleTrails, ToggleAnim, ToggleDegraded, TogglePanel, ToggleFullscreen, Quit }`
  - `pub fn action_for_key(logical: &winit::keyboard::Key, physical: winit::keyboard::PhysicalKey) -> Option<Action>` — table du spec : `" "`→TogglePlay, `n`→Step, `+`/`=`→Faster, `-`→Slower, `r`, `c`, `d`, `e`, `h`, `s`, `l`, `f` (minuscule ou majuscule), `Tab`→TogglePanel, `Escape`→Quit ; `1`/`2`/`3` par caractère **ou** `KeyCode::Digit1..3` physique → ToggleAge/Trails/Anim.
  - `pub fn run()` dans `app.rs`.
  - `panel.rs` : `pub fn draw(ctx: &egui::Context, s: &mut Settings, snap: &Snapshot, speed: Speed, fps: f32, cam: &Camera, notice: Option<&str>) -> Vec<Action>`.

Câblage `app.rs` :
- Démarrage : fenêtre plein écran borderless, `Universe` avec soupe de la taille de l'écran (`fill_random(visible_rect, 0.35, seed)`), en pause, caméra centrée sur (0,0) à 8 px/cellule.
- Souris : clic gauche (hors panneau, `egui` `wants_pointer_input` faux) → `line_cells` depuis la dernière cellule → `SimCommand::Edit` (vivant ou mort selon `tool_erase`) ; clic droit glissé et défilement trackpad (`PixelDelta`) → `pan_px` ; molette (`LineDelta`) et pincement (`PinchGesture`) → `zoom_at` sous le curseur (facteur `1.1^lignes`, `1+delta` pour le pincement).
- Clavier : ignoré si `egui` veut le clavier ; sinon `action_for_key`.
- `Randomize` → `randomize_rect(&cam)` ; si plafonné, `notice` = « Zone limitée à 268 M cellules » pendant 3 s.
- `Recenter` → `cam.fit(bounding_box)` : `bounding_box(&snapshot.tiles)` (Task 3).
- `ToggleDegraded` / effets → `SimCommand::SetEffects(settings.effects_needed())` à chaque changement.
- Panneau : sections Stats (génération, population, FPS, gén/s, zoom), Simulation (pause/play, 1 pas désactivé en marche, curseur sur `SPEEDS`), Grille (randomiser la vue, densité, tout effacer), Dessin (dessiner/gommer), Apparence (style, sélecteur de couleur, variation, 3 effets, mode dégradé) ; libellés avec raccourcis comme dans la maquette `.superpowers/brainstorm/*/content/panel.html`. Réduit : petite barre (état, génération, population, FPS, bouton pour rouvrir).
- `request_redraw` continu (`ControlFlow::Poll`) ; FPS mesuré.

- [ ] **Step 1: Tests qui échouent** dans `src/ui/keys.rs` :
  - `spec_shortcuts` : chaque ligne de la table du spec → l'action attendue (`Key::Named(Space)` pour Espace, `Key::Character("+")`, `"N"` majuscule aussi).
  - `digit_keys_work_on_azerty` : `Key::Character("&")` avec `PhysicalKey::Code(KeyCode::Digit1)` → `ToggleAge` ; `"é"` + `Digit2` → `ToggleTrails` ; `"\""` + `Digit3` → `ToggleAnim`.
  - `unknown_key_is_none` : `"z"` → `None`.
- [ ] **Step 2:** `cargo test ui::keys` → FAIL.
- [ ] **Step 3:** Implémenter `keys.rs`, `panel.rs`, `app.rs`, `main.rs`.
- [ ] **Step 4:** `cargo test` → PASS ; `cargo run --release` puis vérifier à la main chaque point : lancement plein écran en pause avec soupe ; Espace, N (sans effet en marche), +/−, R (zone visible remplacée ; très dézoomé → message), C, D/E et trait rapide continu, molette/pincement sous le curseur, clic droit / deux doigts, H (et H sur univers vide), S, 1/2/3, L, Tab, F, Échap ; clics sur le panneau ne dessinent pas ; redimensionnement / sortie du plein écran OK.
- [ ] **Step 5:** Commit `feat: full app with panel and shortcuts`.

---

### Task 10: Banc de performance et réglage

**Files:**
- Create: `examples/bench_soup.rs`
- Modify : ce que le profilage désigne (probablement `src/sim/step.rs`, `src/sim/universe.rs`, `src/render/mod.rs`)

**Interfaces:**
- Consumes: `Universe`, `CellRect`.

- [ ] **Step 1:** Écrire `examples/bench_soup.rs` : soupe `CellRect{0,0,4096,4096}` densité 0,35 (~5,9 M vivantes), effets éteints, 10 générations d'échauffement puis 200 générations chronométrées ; afficher `gen/s`, population finale, nombre de tuiles. Puis même chose effets allumés.
- [ ] **Step 2:** `cargo run --release --example bench_soup` → noter les chiffres. Objectif : **≥ 30 gén/s effets éteints** sur le M1 Pro.
- [ ] **Step 3:** Si en dessous : profiler (`cargo instruments -t time` ou `samply`), et optimiser les points chauds (allocation de la table à chaque pas — pré-dimensionner `with_capacity_and_hasher` ; regrouper la recherche des 8 voisines ; granularité rayon). Réexécuter jusqu'à l'objectif.
- [ ] **Step 4:** Lancer l'app en `--release` avec R à un dézoom où la soupe fait plusieurs millions de vivantes ; vérifier dans les stats ~60 FPS en déplacement/zoom pendant que la sim tourne en `Max`. Si le FPS baisse : limiter les ré-uploads (n'envoyer que les tuiles visibles changées) ou réduire la résolution de densité.
- [ ] **Step 5:** `cargo test` → PASS ; commit `perf: benchmark and tuning` avec les chiffres mesurés dans le message.
