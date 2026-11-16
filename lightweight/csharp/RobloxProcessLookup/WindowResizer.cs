// Resizes running Roblox windows. Windows and Roblox may refuse very small sizes,
// so this reports the client size that was actually applied.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;

static class WindowResizer
{
    [StructLayout(LayoutKind.Sequential)]
    struct RECT
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [DllImport("user32.dll", SetLastError = true)]
    static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

    [DllImport("user32.dll", SetLastError = true)]
    static extern bool GetClientRect(IntPtr hWnd, out RECT rect);

    [DllImport("user32.dll", SetLastError = true)]
    static extern bool SetWindowPos(IntPtr hWnd, IntPtr insertAfter, int x, int y, int cx, int cy, uint flags);

    const uint SWP_NOZORDER = 0x0004;
    const uint SWP_NOACTIVATE = 0x0010;

    // Sets each Roblox window's client area to width x height. Returns one line per window
    // with the size Windows actually applied.
    public static List<string> Resize(int width, int height)
    {
        var results = new List<string>();

        foreach (Process process in Process.GetProcessesByName("RobloxPlayerBeta"))
        {
            IntPtr hwnd;
            try { hwnd = process.MainWindowHandle; }
            catch (Exception) { continue; }
            if (hwnd == IntPtr.Zero) continue;

            RECT outer, client;
            if (!GetWindowRect(hwnd, out outer) || !GetClientRect(hwnd, out client)) continue;

            // The outer window includes borders and the title bar, so add them back on.
            int borderWidth = (outer.Right - outer.Left) - (client.Right - client.Left);
            int borderHeight = (outer.Bottom - outer.Top) - (client.Bottom - client.Top);

            SetWindowPos(hwnd, IntPtr.Zero, outer.Left, outer.Top, width + borderWidth, height + borderHeight, SWP_NOZORDER | SWP_NOACTIVATE);

            RECT after;
            if (GetClientRect(hwnd, out after))
                results.Add((after.Right - after.Left) + "x" + (after.Bottom - after.Top));
        }

        return results;
    }
}
