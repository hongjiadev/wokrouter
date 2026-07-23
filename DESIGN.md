<!-- SEED: re-run $impeccable document once there's code to capture the actual tokens and components. -->
---
name: WokRouter
description: A restrained runtime-first control surface for a private local AI router.
---

# Design System: WokRouter

## 1. Overview

**Creative North Star: "The Quiet Control Room"**

WokRouter sits beside an editor throughout a developer's workday, often checked in a glance under changing desktop lighting. The interface should feel like a quiet control room: operationally dense where needed, calm at rest, and unmistakably responsive when the daemon changes state. It follows the operating-system color scheme rather than declaring light or dark mode as a brand posture.

The product combines CC Switch's focused desktop-shell practicality with Cockpit Tools' runtime, account, and quota information hierarchy, while remaining its own clean-room product. It explicitly rejects generic AI IDE multi-account management, marketing-dashboard decoration, copied Cockpit Tools language or assets, and configuration-first navigation.

**Key Characteristics:**

- Runtime state is the strongest visual signal on every screen.
- Familiar desktop controls disappear into the task.
- Density is deliberate; decorative containers are rare.
- Errors pair a plain-language cause with one safe recovery action.
- Locale, timezone, RTL, zoom, and reduced motion work without a separate mode.

## 2. Colors

The palette is restrained: neutral system surfaces carry the workspace, while one deep rose-plum family marks primary actions and current selection on no more than 10% of a screen.

### Primary

- **Control Plum:** deep rose/plum anchored near hue 340°; exact OKLCH values will be resolved during implementation. Use it only for primary action, focus, current navigation, and intentional emphasis.

### Secondary

- **Operational Status:** semantic success, warning, error, and information colors will be resolved as separate accessible roles. They never replace text or icons as the sole state signal.

### Neutral

- **System Canvas:** true neutral light and near-black dark surfaces follow the OS preference; no cream, warm paper, tinted glass, or decorative background grid.
- **Layered Surface:** one quiet tonal step separates navigation and active workspace without nested cards.
- **Operational Ink:** high-contrast text; muted text remains readable and uses the surface's neutral family rather than washed-out gray.

**The Ten Percent Rule.** The plum accent occupies at most 10% of a screen. Its rarity makes actions and focus legible.

## 3. Typography

**Display Font:** single technical-humanist sans [font stack to be chosen at implementation]
**Body Font:** the same family [font stack to be chosen at implementation]
**Label/Mono Font:** system monospace for model IDs, ports, versions, request IDs, and protocol literals only

**Character:** One familiar sans family carries headings, controls, and body copy so the interface stays operational rather than editorial. Monospace distinguishes machine identifiers without turning the whole product into a terminal.

### Hierarchy

- **Headline:** compact, semibold, fixed-size heading for the current workspace.
- **Title:** medium-weight section and status heading.
- **Body:** regular text with a 65–75ch maximum for explanations; dense metadata may run wider.
- **Label:** concise control and field labels with normal casing and no decorative tracking.

**The Literal Data Rule.** Never localize, stylize, or directionally reverse model IDs, versions, ports, request IDs, or protocol names.

## 4. Elevation

The system is flat by default. Depth comes from tonal layering and structure; shadow appears only for a transient popover, dialog, or elevated drag/hover state and never decorates every container.

**The State-Only Motion Rule.** Motion communicates loading, transition, success, failure, or focus within 150–250ms. There are no choreographed page entrances, and reduced-motion removes nonessential transitions.

## 6. Do's and Don'ts

### Do:

- **Do** lead every screen with the current runtime state and the next safe action.
- **Do** use standard buttons, navigation, fields, dialogs, focus rings, and disabled/loading states.
- **Do** preserve readable contrast, keyboard order, 200% zoom, narrow-window use, RTL structure, and reduced motion.
- **Do** pair semantic color with text and a shape or icon.

### Don't:

- **Don't** make this a generic AI IDE account/multi-open manager or imitate Cockpit Tools code, copy, or assets.
- **Don't** use repeated identical card grids, glassmorphism, gradient text, neon AI purple, decorative grid backgrounds, or wide ghost-card shadows.
- **Don't** put configuration forms before daemon health and recovery.
- **Don't** use side-stripe accents, cards above 16px radius, decorative motion, or color as the only status signal.
