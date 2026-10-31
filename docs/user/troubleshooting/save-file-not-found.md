---
title: Save As says "File not found" in Documents or Pictures
description: On Windows, saving or exporting into Documents, Pictures or Desktop can fail with "File not found" when Windows Security's Controlled folder access blocks PhotoCraft. Allow PhotoCraft through it, or save to another folder.
order: 25
verified: 0.14.0
---

The most likely cause is Windows Security's **Controlled folder access**, which stops apps it doesn't know from writing to Documents, Pictures and Desktop. Allow PhotoCraft through it, or save to a folder outside those.

## Symptoms

- On Windows 10 or 11, you choose **File › Save As…** or export, pick a folder such as Documents or Pictures, and click **Save**.
- The Windows save dialog shows "File not found. Check the file name and try again." and nothing is saved ([issue 5750](https://github.com/storytold/photocraft/issues/5750), [issue 7600](https://github.com/storytold/photocraft/issues/7600), [issue 7999](https://github.com/storytold/photocraft/issues/7999)).
- Saving the same file to another folder, such as `C:\Temp`, works.

## Fix

First, check that this is the cause:

1. Open **Windows Security** from the Start menu.
2. Choose **Virus & threat protection**, then **Protection history**.
3. Look for a "Protected folder access blocked" entry that names `photocraft.exe`. If there is one, continue below.

Then allow PhotoCraft:

1. In **Windows Security**, choose **Virus & threat protection**.
2. Under **Ransomware protection**, click **Manage ransomware protection**.
3. Click **Allow an app through Controlled folder access**, then **Add an allowed app**.
4. Pick PhotoCraft from **Recently blocked apps**, or choose **Browse all apps** and select `photocraft.exe`. The installer puts it in `C:\Program Files\PhotoCraft` unless you chose another folder.
5. Save again in PhotoCraft.

If your PC is managed by work or school, you may not be allowed to change this. Save to a folder outside Documents, Pictures and Desktop instead.

## Why this happens

Controlled folder access is part of Windows ransomware protection. It blocks writes to protected folders from apps that Windows doesn't recognize, and a new app like PhotoCraft isn't recognized yet. Before the save dialog returns your choice, Windows test-creates the file. When that test is blocked, the dialog reports "File not found" rather than saying access was denied, so PhotoCraft never gets to save. Other image editors show the same message when they are blocked.

## Still stuck?

If nothing shows in **Protection history**, or allowing PhotoCraft doesn't help, add your Windows version, PhotoCraft version and the folder you saved to in [issue 5750](https://github.com/storytold/photocraft/issues/5750). See [Get help with a problem](get-help.md) for what to include.
