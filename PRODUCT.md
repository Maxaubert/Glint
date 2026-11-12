# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(The real app is a native Windows 11 program; design exploration happens as HTML mockups. The editor UI is
built in Slint 1.18 with its software renderer, the daemon in plain Win32/GDI.)

## Users
Public first (owner, 2026-11-12): people who find Glint on GitHub and install it as their Windows snipping
tool. They press Win+Shift+S, drag an area, and usually want one quick mark (an arrow or a circle) and then
paste the picture somewhere. The owner uses it daily the same way.

## Product Purpose
A Windows 11 snipping tool that is faster and nicer than the built-in one: snip, preview card, a small
annotation editor, copy or save. Success: a snip with an arrow is on the clipboard in a few seconds, and
the tool costs almost no memory while idle.

## Positioning
Tiny idle footprint (about 2 MB) with a full annotation editor one click away; works under the Wind
magnifier (snips the real desktop while zoomed).

## Operating Context
Opened from the preview card after a snip, on a dark Windows 11 desktop, often at 225 % display scale.
Used for seconds at a time: draw one or two marks, Copy (or Enter), close.

## Capabilities and Constraints
- Tools: select, arrow, rectangle, ellipse, highlighter, pen, text, numbered step, blur, crop (iOS style).
- 6 colours, 7 stroke sizes shared by all tools; undo/redo; Copy, Save (PNG, JPEG, WebP).
- Copy animates its success state; the owner wants a better Copy success animation.
- Slint software renderer: animations of position, size, opacity, colour and font size are cheap; heavy
  blur or GPU effects are not available in the real app.
- No status bar; the image size sits in the title bar.

## Brand Commitments
Name "Glint". Accent purple #8b5cf6 used so far (not confirmed as binding). App icon in `assets/glint.ico`.

## Evidence on Hand
README screenshots in `assets/`. No testimonials or user numbers; do not invent any.

## Product Principles
- Get out of the way: the picture is the content, the chrome is quiet.
- One or two marks then done: the most common path is the shortest.
- Clear to a first-time user without reading anything.
- Small and fast stays a feature.
