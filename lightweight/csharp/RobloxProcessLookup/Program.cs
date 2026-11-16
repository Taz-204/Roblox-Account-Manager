// Command-line helper for the lightweight Roblox account manager. Windows only.
//
//   RobloxProcessLookup.exe                 Print running Roblox game processes as JSON.
//   RobloxProcessLookup.exe protect         Read a .ROBLOSECURITY cookie from stdin and print it encrypted (base64).
//   RobloxProcessLookup.exe launch <placeId> [jobId]  Read an encrypted cookie from stdin and start the game.
//   RobloxProcessLookup.exe login           Open a Roblox login window. Prints the encrypted cookie, username, and user id.
//   RobloxProcessLookup.exe fps <n>         Set the Roblox FPS cap (0 removes it).
//   RobloxProcessLookup.exe graphics <1-10>  Set the Roblox graphics quality override (0 removes it).
//   RobloxProcessLookup.exe flags           Read a JSON object of client flags from stdin and write them to Roblox's settings.
//   RobloxProcessLookup.exe spread          Give each running Roblox window its own group of CPU cores. Prints the count.
//   RobloxProcessLookup.exe resize <width> <height>  Resize every Roblox window. Prints the size applied for each.
//   RobloxProcessLookup.exe priority [normal]  Set the priority of running Roblox windows (Below Normal, or Normal). Prints the count.
//   RobloxProcessLookup.exe avatar <userId> <path>  Save the account's headshot as a PNG.
//   RobloxProcessLookup.exe multi-instance  Hold Roblox's single-instance lock. Runs until ended.
//
// Errors go to stderr with exit code 1. Cookies are never printed.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Management;
using System.Text;
using System.Threading;

static class Program
{
    [STAThread]
    static int Main(string[] args)
    {
        try
        {
            if (args.Length == 0) return ListProcesses();

            switch (args[0])
            {
                case "protect":
                    WriteValues(RobloxLauncher.ProtectWithAccount(ReadStdinLine()));
                    return 0;

                case "login":
                    string cookie = BrowserLogin.Run();
                    if (string.IsNullOrEmpty(cookie)) return Fail("login was cancelled");
                    WriteValues(RobloxLauncher.ProtectWithAccount(cookie));
                    return 0;

                case "fps":
                    if (args.Length < 2) return Fail("usage: fps <number>, 0 removes the cap");
                    int fps;
                    if (!int.TryParse(args[1], NumberStyles.None, CultureInfo.InvariantCulture, out fps) || fps < 0)
                        return Fail("fps must be a whole number, 0 or more");
                    Console.WriteLine(RobloxLauncher.SetFps(fps));
                    return 0;

                case "graphics":
                    if (args.Length < 2) return Fail("usage: graphics <1-10>, 0 removes the override");
                    int level;
                    if (!int.TryParse(args[1], NumberStyles.None, CultureInfo.InvariantCulture, out level) || level < 0 || level > 10)
                        return Fail("graphics level must be 0 to 10");
                    Console.WriteLine(RobloxLauncher.SetGraphicsLevel(level));
                    return 0;

                case "flags":
                    Console.WriteLine(RobloxLauncher.ApplyCustomFlags(Console.In.ReadToEnd()));
                    return 0;

                case "spread":
                    Console.WriteLine(RobloxLauncher.SpreadAcrossCores());
                    return 0;

                case "resize":
                    if (args.Length < 3) return Fail("usage: resize <width> <height>");
                    int width, height;
                    if (!int.TryParse(args[1], NumberStyles.None, CultureInfo.InvariantCulture, out width) || width < 1 || width > 4000
                        || !int.TryParse(args[2], NumberStyles.None, CultureInfo.InvariantCulture, out height) || height < 1 || height > 4000)
                        return Fail("width and height must be whole numbers from 1 to 4000");
                    foreach (string size in WindowResizer.Resize(width, height)) Console.WriteLine(size);
                    return 0;

                case "priority":
                    // "normal" sets Normal priority. Anything else sets Below Normal.
                    bool normal = args.Length > 1 && args[1] == "normal";
                    Console.WriteLine(RobloxLauncher.SetRobloxPriority(normal));
                    return 0;

                case "avatar":
                    if (args.Length < 3) return Fail("usage: avatar <userId> <path>");
                    long avatarUserId;
                    if (!long.TryParse(args[1], NumberStyles.None, CultureInfo.InvariantCulture, out avatarUserId))
                        return Fail("user id must be a number");
                    RobloxLauncher.DownloadAvatar(avatarUserId, args[2]);
                    return 0;

                case "multi-instance":
                    return HoldRobloxSingleton();

                case "serve":
                    return Serve();

                case "launch":
                    if (args.Length < 2) return Fail("usage: launch <placeId> [jobId]");
                    long placeId;
                    if (!long.TryParse(args[1], NumberStyles.None, CultureInfo.InvariantCulture, out placeId))
                        return Fail("place id must be a number");
                    string jobId = args.Length > 2 ? args[2] : null;
                    int pid = RobloxLauncher.Launch(RobloxLauncher.Unprotect(ReadStdinLine()), placeId, jobId);
                    Console.WriteLine("{\"ok\":true,\"pid\":" + pid + "}");
                    return 0;

                default:
                    return Fail("unknown command: " + args[0]);
            }
        }
        catch (Exception ex)
        {
            return Fail(ex.Message);
        }
    }

    // One value per line: encrypted cookie, username, user id (empty if unknown).
    static void WriteValues(string[] values)
    {
        foreach (string value in values) Console.WriteLine(value);
    }

    // Holding Roblox's single-instance event stops Roblox from blocking a second client.
    // Runs until the process is ended.
    static int HoldRobloxSingleton()
    {
        using (var singleton = new EventWaitHandle(false, EventResetMode.AutoReset, "ROBLOX_singletonEvent"))
        {
            Console.WriteLine("holding");
            Console.Out.Flush();
            Thread.Sleep(Timeout.Infinite);
            GC.KeepAlive(singleton);
        }
        return 0;
    }

    // Keeps running and launches clients on request, one per line on stdin:
    //   placeId<TAB>jobId<TAB>encryptedCookie
    // Each reply is one line: {"ok":true,"pid":N} or {"ok":false,"error":"..."}.
    // Staying alive means .NET starts once, and HTTPS connections to Roblox are reused.
    static int Serve()
    {
        string line;
        while ((line = Console.In.ReadLine()) != null)
        {
            string reply;
            try
            {
                string[] parts = line.Split('\t');
                if (parts.Length < 3) throw new InvalidOperationException("bad launch request");
                long placeId;
                if (!long.TryParse(parts[0], NumberStyles.None, CultureInfo.InvariantCulture, out placeId))
                    throw new InvalidOperationException("place id must be a number");
                string jobId = parts[1].Length > 0 ? parts[1] : null;
                int pid = RobloxLauncher.Launch(RobloxLauncher.Unprotect(parts[2]), placeId, jobId);
                reply = "{\"ok\":true,\"pid\":" + pid + "}";
            }
            catch (Exception e)
            {
                string message = e.Message.Replace("\\", "\\\\").Replace("\"", "\\\"").Replace("\r", " ").Replace("\n", " ");
                reply = "{\"ok\":false,\"error\":\"" + message + "\"}";
            }
            Console.Out.Write(reply + "\n");
            Console.Out.Flush();
        }
        return 0;
    }

    static string ReadStdinLine()
    {
        string line = Console.In.ReadLine();
        if (string.IsNullOrWhiteSpace(line)) throw new InvalidOperationException("no input on stdin");
        return line.Trim();
    }

    static int Fail(string message)
    {
        Console.Error.WriteLine("error: " + message);
        return 1;
    }

    static int ListProcesses()
    {
        var items = new List<string>();

        foreach (Process process in Process.GetProcessesByName("RobloxPlayerBeta"))
        {
            try
            {
                if (process.HasExited) continue;

                string start = process.StartTime.ToUniversalTime().ToString("o");
                string title = process.MainWindowTitle;
                string commandLine = GetCommandLine(process.Id);

                items.Add("{\"pid\":" + process.Id
                    + ",\"start\":" + JsonString(start)
                    + ",\"window_title\":" + JsonString(title)
                    + ",\"command_line\":" + JsonString(commandLine) + "}");
            }
            catch (Exception)
            {
                // The process exited or we don't have access to it. Skip it.
            }
        }

        Console.WriteLine("[" + string.Join(",", items) + "]");
        return 0;
    }

    static string GetCommandLine(int processId)
    {
        using (var searcher = new ManagementObjectSearcher(
            "SELECT CommandLine FROM Win32_Process WHERE ProcessId = " + processId))
        using (var results = searcher.Get())
        {
            foreach (ManagementBaseObject obj in results)
            {
                object value = obj["CommandLine"];
                return value == null ? null : value.ToString();
            }
            return null;
        }
    }

    // Escapes a string as a JSON string literal. Null becomes null.
    static string JsonString(string value)
    {
        if (value == null) return "null";

        var sb = new StringBuilder("\"");
        foreach (char c in value)
        {
            switch (c)
            {
                case '"': sb.Append("\\\""); break;
                case '\\': sb.Append("\\\\"); break;
                case '\n': sb.Append("\\n"); break;
                case '\r': sb.Append("\\r"); break;
                case '\t': sb.Append("\\t"); break;
                default:
                    if (c < 0x20) sb.AppendFormat("\\u{0:x4}", (int)c);
                    else sb.Append(c);
                    break;
            }
        }
        sb.Append('"');
        return sb.ToString();
    }
}
