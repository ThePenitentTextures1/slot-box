# The Shelf

## Cartridge View controls

These are the default shelf controls. The collection controls also apply to
games on the Box View carousel. The four shelf bindings can be changed in
Settings → Controls → Shelf.

### Game as a collection item

| Button | Does |
| --- | --- |
| <kbd>Left</kbd> / <kbd>Right</kbd> | Move along the shelf |
| <kbd>Up</kbd> / <kbd>Down</kbd> | Jump to the previous or next letter |
| <kbd>L1</kbd> / <kbd>R1</kbd> | Switch shelves |
| <kbd>Y</kbd> | Favorite / unfavorite the selected game |
| <kbd>X</kbd> | Remove the selected game from Recents, when on the Recents shelf |

### Game as a playable cartridge

| Button | Does |
| --- | --- |
| Tap <kbd>A</kbd> | Insert the cartridge, using the auto-load setting |
| Hold <kbd>A</kbd> | Insert the cartridge without auto-loading a save state |
| <kbd>START</kbd> | Choose the emulator |
| <kbd>SELECT</kbd> | Open Cartridge Studio |

These cartridge actions apply to a selected cartridge in Cartridge View or
Open Box View. <kbd>B</kbd> has no action on the Cartridge View carousel.

### Slot controls

| Button | Does |
| --- | --- |
| <kbd>MENU</kbd> | Open settings |

Volume, display, power, and other system shortcuts are separate from the
selected game; see [Controls](controls.md).

## Box View

When Box View is enabled and a game has box art, the carousel has these
box-specific controls in addition to the collection controls:

| Button | Does |
| --- | --- |
| Tap <kbd>A</kbd> | Open the box and its contents |
| Hold <kbd>A</kbd> | Skip Open Box View and start fresh |
| <kbd>START</kbd> | Skip Open Box View and play using the auto-load setting |
| <kbd>SELECT</kbd> | Inspect the box |
| <kbd>B</kbd> | No action on the carousel; returns to it from Open Box View |

The cartridge is selected first in Open Box View. Press <kbd>Down</kbd> to select the box and
<kbd>Up</kbd> to return to the cartridge. With the cartridge selected, the
cartridge controls above apply. Open the box to change the emulator or use
Cartridge Studio. Games without box art retain the regular cartridge controls
directly on the carousel.

With the box selected, <kbd>A</kbd> flips it, <kbd>R1</kbd> zooms in, and
<kbd>L1</kbd> zooms out. While zoomed, hold a direction to pan and press
<kbd>B</kbd> to reset the zoom. At normal zoom, <kbd>B</kbd> or
<kbd>Left</kbd>/<kbd>Right</kbd> closes the box. On the carousel,
<kbd>SELECT</kbd> inspects the box; the same flip, zoom and pan controls apply,
and <kbd>B</kbd> returns to the carousel. If the box is facing backward when
closed, it flips to the front before the cartridge is put away.

## The shelves

There's one shelf per system, along with two special shelves, in this order:

1. Recents, if enabled in settings
2. Favorites, once you've hearted something
3. Game Boy Advance
4. Game Boy Advance Video
5. Game Boy Color
6. Game Boy

slot does not show empty shelves. If you only have GBA games, you only get the GBA shelf.

Game Boy Advance Video carts can be optionally placed in `Games/GBAV`. Here they get their own shelf and Game Boy Player support is disabled when using mGBA.

Within a shelf, games are sorted by name. Names starting with a number come first.

slot remembers which shelf and cart you were on when it gets powered off and will return there on the next boot. If you powered down while still in game it will resume that game on the next boot.

## Favorites

Press <kbd>Y</kbd> to heart a cart. It shows up on the Favorites shelf alongside any other favorites across all three platforms. Press <kbd>Y</kbd> again to take the heart off.

## Recents

Turn on Recents in Settings → General to get a shelf of the last 10 games you played.

Press <kbd>X</kbd> on the Recents shelf to remove the selected game.

## Game titles

Turn on Game Titles in Settings → General and they will appear above the slot.

## Adding games

New games, labels and cart shells are picked up when slot boots.

See [Storage Reference](storage.md) for where games go and [Customization](customizing.md) for labels / shells.
