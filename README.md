# Screensplitter

**Note:** This repository is archived and no longer maintained.

---

This project is a product of its time. During the initial COVID-19 lockdown, while transitioning to remote work, I encountered a limitation with screen sharing on Linux using a dual-monitor setup. At the time, it was only possible to share either the entire desktop (i.e., both monitors) or individual application windows. There was no straightforward way to share just one screen.

As a workaround (and as an opportunity to play with Rust) I created this small application. It reads the contents of one screen and mirrors it into a regular X11 window. This window could then be selected in Google Chrome as if it were a standard application window, enabling effective screen sharing of a single monitor.

Of course, this approach has since become obsolete. Screen sharing tools improved rapidly during 2020, and just weeks after I had a working prototype, Chrome added native support for sharing individual screens on Linux.

---
