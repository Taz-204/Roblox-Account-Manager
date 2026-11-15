// Prints the running Roblox game processes as a JSON array, for example:
//   [{"pid":1234,"start":"2026-11-15T16:00:00.0000000+00:00","window_title":"Roblox","command_line":"..."}]
// Windows only. Reads each process's command line through WMI once per run.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Management;
using System.Text;

static class Program
{
    static int Main()
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
