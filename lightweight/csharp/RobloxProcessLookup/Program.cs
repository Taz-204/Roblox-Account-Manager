// Command-line helper for the lightweight Roblox account manager. Windows only.
//
//   RobloxProcessLookup.exe                 Print running Roblox game processes as JSON.
//   RobloxProcessLookup.exe protect         Read a .ROBLOSECURITY cookie from stdin and print it encrypted (base64).
//   RobloxProcessLookup.exe launch <placeId> [jobId]  Read an encrypted cookie from stdin and start the game.
//   RobloxProcessLookup.exe login           Open a Roblox login window and print the encrypted cookie.
//
// Errors go to stderr with exit code 1. Cookies are never printed.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Management;
using System.Text;

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
                    Console.WriteLine(RobloxLauncher.Protect(ReadStdinLine()));
                    return 0;

                case "login":
                    string cookie = BrowserLogin.Run();
                    if (string.IsNullOrEmpty(cookie)) return Fail("login was cancelled");
                    Console.WriteLine(RobloxLauncher.Protect(cookie));
                    return 0;

                case "launch":
                    if (args.Length < 2) return Fail("usage: launch <placeId> [jobId]");
                    long placeId;
                    if (!long.TryParse(args[1], NumberStyles.None, CultureInfo.InvariantCulture, out placeId))
                        return Fail("place id must be a number");
                    string jobId = args.Length > 2 ? args[2] : null;
                    RobloxLauncher.Launch(RobloxLauncher.Unprotect(ReadStdinLine()), placeId, jobId);
                    Console.WriteLine("{\"ok\":true}");
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
