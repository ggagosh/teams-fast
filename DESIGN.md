---
name: TeamsFast
description: A native Teams messenger composed from GPUI Kit on GPUI Fast.
colors:
  dark-background: "#17191e"
  dark-sidebar: "#202329"
  dark-surface: "#2a2e36"
  dark-foreground: "#e9edf4"
  dark-secondary-text: "#a4adbb"
  dark-border: "#343a45"
  dark-primary: "#8aaaf8"
  dark-muted: "#23272e"
  dark-accent: "#303e58"
  dark-primary-foreground: "#172442"
  dark-button-primary: "#496fd0"
  light-background: "#f5f6f8"
  light-sidebar: "#eceff3"
  light-surface: "#ffffff"
  light-foreground: "#242b36"
  light-secondary-text: "#637084"
  light-border: "#dbe0e8"
  light-primary: "#365fc7"
  light-muted: "#e7ebf1"
  light-accent: "#dce5f7"
typography:
  body:
    fontSize: "14px"
  chat-row:
    fontSize: "13px"
  author:
    fontSize: "11px"
  timestamp:
    fontSize: "10px"
components:
  conversation-header:
    height: "56px"
  chat-row:
    height: "58px"
  avatar-chat:
    size: "30px"
  avatar-message:
    size: "28px"
---

# Design System: TeamsFast

## Overview

TeamsFast uses GPUI Kit components on GPUI Fast, with gray surfaces and blue accents applied through Kit's semantic theme. Compact navigation leads into neutral incoming bubbles and right-aligned tinted own messages. A compact composer stays below the transcript; Settings has its own native window.

The implementation lives in `src/ui.rs`, `src/app.rs`, `src/dialogs.rs`, `src/preferences.rs`, and `src/main.rs`. [PRODUCT.md](PRODUCT.md) defines scope. Frontmatter records the colors assigned by `ui::apply_theme` and explicit application measurements; Kit owns the remaining control tokens and rendering.

**Key Characteristics:**

- Kit components with gray/blue semantic theme values.
- Resizable conversation navigation and a virtualized transcript.
- Neutral incoming bubbles, tinted own messages, and inline formatted links.
- Dark appearance by default, with a light appearance setting.

## Colors

`ui::apply_theme` updates Kit's `Theme` for dark and light appearances. Components continue consuming `ActiveTheme` roles; no custom-painted widget system is introduced. The frontmatter records the actual overrides.

- `background` and `foreground` define the conversation canvas and reading text.
- `sidebar` separates navigation; `border` defines pane and header boundaries.
- `secondary` provides incoming bubble surfaces; `muted` supports pending content and date labels.
- `muted_foreground` serves previews, author labels, times, and secondary status.
- `primary` provides blue tint/caret roles; `button_primary` supplies the send button. Primary button text is white in both themes.
- `accent` is the selected-list surface; list hover uses `muted`.
- `success` marks connected state; `danger` presents errors.

Keep controls tied to Kit's theme roles in both appearances and retain status text alongside color.

## Typography

Use Kit's default native/system typography. The main view sets body text to the frontmatter size; Kit's small and extra-small styles serve labels, timestamps, previews, and shortcuts. Medium/semibold weights distinguish conversation titles and unread activity.

Noto Sans Georgian is registered with GPUI's text system. Keep the bundled font and verify actual Georgian rendering/input in the native app; registration alone is not a glyph-coverage guarantee.

## Layout

The main window starts at 1120 × 780 logical pixels with a 760 × 520 minimum. Compact mode starts at 780 × 580. A Kit `TitleBar` spans the window, above a horizontal resizable layout. The sidebar starts at 290 pixels and ranges from 240 to 400.

The conversation contains its compact header, variable-height `MessageScroller`, pending/error feedback, and bottom composer. Bubbles cap at 640 pixels; own messages align right. The composer grows from one to six rows inside a bordered input surface, with its Send icon alongside it. Retain minimum-zero flex sizing so the transcript scrolls while the header and composer remain usable.

Settings opens at 780 × 650 with a 640 × 480 minimum and a 185-pixel navigation column. It has Account, Notifications, Appearance, and Advanced pages.

## Elevation & Depth

Use Kit's normal theme surfaces, borders, dialog treatment, and message-scroller bottom fade. The application has no separate shadow or elevation system.

## Shapes

Use Kit's control, avatar, badge, bubble, attachment, and dialog shapes. Chat rows, pending-send containers, and the composer use Kit's large rounded style. Date separators are compact pills. Icons come from Kit's Lucide icon set; do not recreate them with custom painting.

## Components

- **Window chrome:** Kit `TitleBar` and its window options preserve native window controls. The titlebar includes a ghost Settings action.
- **Conversation navigation:** `Input` for search, `ListItem` for selected rows, `Avatar` and `Badge` for identity/unread counts. Titles/previews truncate; unread titles gain weight. Accessibility labels include unread and muted state.
- **Transcript:** `MessageScroller`, `MessageGroup`, `Message`, and `Bubble` provide grouped rows with pill date separators. Incoming messages have neutral bubbles and grouped avatars; own messages use right-aligned tinted bubbles. `TextView` renders escaped Markdown with inline HTTP(S) links. Small Kit `Attachment` cards show metadata and open the conversation in Teams.
- **Composer:** a frameless Kit `Textarea` grows from one to six rows inside the compact input surface. Enter sends and Shift+Enter adds a newline; the accessible label and Send tooltip explain the shortcut. A primary icon button, pending text/spinner, and visible errors communicate delivery state. The draft remains until acknowledgement.
- **Dialogs:** sign-in, new-chat, and disconnect use Kit dialogs, inputs, checkboxes, and buttons.
- **Settings:** Kit `Settings`, `SettingPage`, `SettingGroup`, `SettingItem`, and switch fields compose the separate native window. Connection diagnostics stay on Advanced.

## Do's and Don'ts

### Do:

- **Do** use existing GPUI Kit controls and theme roles.
- **Do** preserve native text input, selection, focus, and window behavior.
- **Do** keep Enter/Shift+Enter behavior, accessible labels, and tooltips consistent.
- **Do** check both appearances, Georgian text, and narrow windows in the runnable app.

### Don't:

- **Don't** restore the old egui palette or custom-painted controls.
- **Don't** invent a second component, icon, or theme system.
- **Don't** clear an unacknowledged draft or automatically repeat a message POST.
- **Don't** expose tokens, keys, or real message content in diagnostics.
