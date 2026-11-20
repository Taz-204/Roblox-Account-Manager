// Account cookie protection, account lookup, avatars, FPS cap, and game launching.
// The launch flow matches the original Account.cs in the C# app:
//   1. POST auth.roblox.com/v1/authentication-ticket/ -> 403 with an X-CSRF-TOKEN header
//   2. POST again with that token -> rbx-authentication-ticket header
//   3. Start RobloxPlayerBeta.exe with the ticket (or open the roblox-player: link as a fallback).
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Web;
using System.Web.Script.Serialization;

static class RobloxLauncher
{
    const string AuthBase = "https://auth.roblox.com/";
    const string UsersBase = "https://users.roblox.com/";
    const string ThumbnailsBase = "https://thumbnails.roblox.com/";
    const string Referer = "https://www.roblox.com/games/4924922222/Brookhaven-RP";
    const string FpsKey = "DFIntTaskSchedulerTargetFps";
    const string GraphicsKey = "DFIntDebugFRMQualityLevelOverride";
    const string CustomKeysFile = "rbx-alt-manager-flags.txt";

    static RobloxLauncher()
    {
        ServicePointManager.SecurityProtocol |= SecurityProtocolType.Tls12;
        // .NET Framework allows only 2 connections per host by default. Raise it for the long-running launcher.
        ServicePointManager.DefaultConnectionLimit = 20;
    }

    // Encrypts the cookie so only this Windows user account can decrypt it.
    public static string Protect(string cookie)
    {
        byte[] data = Encoding.UTF8.GetBytes(cookie);
        byte[] encrypted = ProtectedData.Protect(data, null, DataProtectionScope.CurrentUser);
        return Convert.ToBase64String(encrypted);
    }

    public static string Unprotect(string blob)
    {
        byte[] encrypted = Convert.FromBase64String(blob);
        byte[] data = ProtectedData.Unprotect(encrypted, null, DataProtectionScope.CurrentUser);
        return Encoding.UTF8.GetString(data);
    }

    // Returns { encrypted cookie, username, user id }. Name and id are "" if Roblox didn't say.
    public static string[] ProtectWithAccount(string cookie)
    {
        string blob = Protect(cookie);
        string name = "";
        string id = "";

        Dictionary<string, object> user = GetAuthenticatedUser(cookie);
        if (user != null)
        {
            object value;
            if (user.TryGetValue("name", out value) && value != null) name = value.ToString();
            if (user.TryGetValue("id", out value) && value != null) id = value.ToString();
        }
        return new[] { blob, name, id };
    }

    static Dictionary<string, object> GetAuthenticatedUser(string cookie)
    {
        HttpWebResponse response = Send("GET", UsersBase + "v1/users/authenticated", cookie, null);
        if (response.StatusCode != HttpStatusCode.OK) return null;

        using (var reader = new StreamReader(response.GetResponseStream()))
        {
            return new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(reader.ReadToEnd());
        }
    }

    // Downloads the account's headshot to destination (a PNG file). Thumbnails are public, so no cookie is sent.
    public static void DownloadAvatar(long userId, string destination)
    {
        string url = ThumbnailsBase + "v1/users/avatar-headshot?userIds=" + userId + "&size=150x150&format=Png&isCircular=false";
        HttpWebResponse listing = Send("GET", url, null, null);
        if (listing.StatusCode != HttpStatusCode.OK)
            throw new InvalidOperationException("Roblox returned HTTP " + (int)listing.StatusCode + " for the avatar.");

        string imageUrl = null;
        using (var reader = new StreamReader(listing.GetResponseStream()))
        {
            var root = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(reader.ReadToEnd());
            var entries = root != null && root.ContainsKey("data") ? root["data"] as object[] : null;
            var first = entries != null && entries.Length > 0 ? entries[0] as Dictionary<string, object> : null;
            if (first != null && first.ContainsKey("imageUrl")) imageUrl = first["imageUrl"] as string;
        }

        if (string.IsNullOrEmpty(imageUrl))
            throw new InvalidOperationException("Roblox has no avatar image yet. Try again in a moment.");

        HttpWebResponse image = Send("GET", imageUrl, null, null);
        if (image.StatusCode != HttpStatusCode.OK)
            throw new InvalidOperationException("Could not download the avatar image (HTTP " + (int)image.StatusCode + ").");

        Directory.CreateDirectory(Path.GetDirectoryName(destination));
        using (Stream source = image.GetResponseStream())
        using (FileStream target = File.Create(destination))
        {
            source.CopyTo(target);
        }
    }

    // Starts the game for the account. An empty jobId joins any server; a job ID joins that server.
    public static int Launch(string cookie, long placeId, string jobId)
    {
        string csrf = GetCsrfToken(cookie);
        string ticket = GetTicket(cookie, csrf);

        Random random = new Random();
        string browserTrackerId = random.Next(100000, 175000).ToString() + random.Next(100000, 900000).ToString();
        long launchTime = (long)Math.Floor((DateTime.UtcNow - new DateTime(1970, 1, 1)).TotalSeconds * 1000);

        bool hasJob = !string.IsNullOrEmpty(jobId);
        string placeLauncher = "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGame" + (hasJob ? "Job" : "")
            + "&browserTrackerId=" + browserTrackerId
            + "&placeId=" + placeId
            + (hasJob ? "&gameId=" + HttpUtility.UrlEncode(jobId) : "")
            + "&isPlayTogetherGame=false";

        // Preferred: start RobloxPlayerBeta.exe directly, the same way the original app's
        // "old join" path does. This fails with a clear error if Roblox isn't found.
        string versionFolder = FindLatestVersionFolder();
        if (versionFolder != null)
        {
            string exe = Path.Combine(versionFolder, "RobloxPlayerBeta.exe");
            var info = new ProcessStartInfo(exe)
            {
                Arguments = "--app -t " + ticket + " -j \"" + placeLauncher + "\"",
                UseShellExecute = false
            };
            Process started = Process.Start(info);
            if (started == null) throw new InvalidOperationException("Windows did not start RobloxPlayerBeta.exe.");
            return started.Id;
        }

        // Fallback: the roblox-player: link, which only works if Roblox registered that link.
        string link = "roblox-player:1+launchmode:play+gameinfo:" + ticket
            + "+launchtime:" + launchTime
            + "+placelauncherurl:" + HttpUtility.UrlEncode(placeLauncher)
            + "+browsertrackerid:" + browserTrackerId
            + "+robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp";

        Process.Start(new ProcessStartInfo { FileName = link, UseShellExecute = true });
        return 0;
    }

    // Sets the FPS cap in Roblox's client settings. 0 removes the cap.
    public static string SetFps(int fps)
    {
        return SetClientSetting(FpsKey, fps > 0 ? (object)fps : null);
    }

    // Graphics quality override (1 = lowest, 10 = highest). 0 removes the override.
    public static string SetGraphicsLevel(int level)
    {
        return SetClientSetting(GraphicsKey, level > 0 ? (object)level : null);
    }

    // Writes the user's own client flags into ClientAppSettings.json. Flags written last time
    // (listed in a sidecar file) are removed first, so deleting a flag in the editor really removes it.
    public static string ApplyCustomFlags(string json)
    {
        var serializer = new JavaScriptSerializer();
        var custom = string.IsNullOrWhiteSpace(json)
            ? new Dictionary<string, object>()
            : serializer.Deserialize<Dictionary<string, object>>(json);
        if (custom == null) throw new InvalidOperationException("Flags must be a JSON object like { \"FlagName\": value }.");

        string versionFolder = FindLatestVersionFolder();
        if (versionFolder == null) throw new InvalidOperationException("Roblox is not installed yet. Start Roblox once, then try again.");

        string settingsFolder = Path.Combine(versionFolder, "ClientSettings");
        Directory.CreateDirectory(settingsFolder);
        string file = Path.Combine(settingsFolder, "ClientAppSettings.json");
        string keysFile = Path.Combine(settingsFolder, CustomKeysFile);

        var settings = ReadClientSettings(file, serializer);
        if (File.Exists(keysFile))
        {
            foreach (string key in File.ReadAllLines(keysFile))
            {
                if (key.Length > 0 && key != FpsKey && key != GraphicsKey) settings.Remove(key);
            }
        }
        foreach (var pair in custom) settings[pair.Key] = pair.Value;

        File.WriteAllText(file, serializer.Serialize(settings));
        File.WriteAllLines(keysFile, custom.Keys.ToArray());
        return file;
    }

    // Spreads running Roblox windows across the CPU cores. Each window gets its own group of
    // cores (one core each if there are more windows than cores). Returns how many were set.
    public static int SpreadAcrossCores()
    {
        int cores = Math.Min(Environment.ProcessorCount, 62);
        var windows = Process.GetProcessesByName("RobloxPlayerBeta")
            .Where(p => SafeMainWindow(p) != IntPtr.Zero)
            .OrderBy(p => SafeStart(p))
            .ToList();
        if (windows.Count == 0 || cores <= 0) return 0;

        int groupSize = Math.Max(1, cores / windows.Count);
        int changed = 0;
        for (int i = 0; i < windows.Count; i++)
        {
            long mask = 0;
            for (int c = 0; c < groupSize; c++)
            {
                mask |= 1L << ((i * groupSize + c) % cores);
            }
            try
            {
                windows[i].ProcessorAffinity = (IntPtr)mask;
                changed++;
            }
            catch (Exception)
            {
                // Process exited or we don't have access. Skip it.
            }
        }
        return changed;
    }

    static IntPtr SafeMainWindow(Process p)
    {
        try { return p.MainWindowHandle; } catch (Exception) { return IntPtr.Zero; }
    }

    static DateTime SafeStart(Process p)
    {
        try { return p.StartTime; } catch (Exception) { return DateTime.MaxValue; }
    }

    // Lowers the CPU priority of every running Roblox window so they yield to other work.
    // Returns how many windows were changed.
    public static int SetRobloxPriority(bool normal)
    {
        int changed = 0;
        foreach (Process process in Process.GetProcessesByName("RobloxPlayerBeta"))
        {
            try
            {
                process.PriorityClass = normal ? ProcessPriorityClass.Normal : ProcessPriorityClass.BelowNormal;
                changed++;
            }
            catch (Exception)
            {
                // Process exited or we don't have access. Skip it.
            }
        }
        return changed;
    }

    // Writes one client setting. A null value removes it. Returns the file written.
    // This is a client-wide setting, so it applies to every Roblox instance. Roblox must restart to pick it up.
    static string SetClientSetting(string key, object value)
    {
        string versionFolder = FindLatestVersionFolder();
        if (versionFolder == null)
            throw new InvalidOperationException("Roblox is not installed yet. Start Roblox once, then try again.");

        string settingsFolder = Path.Combine(versionFolder, "ClientSettings");
        Directory.CreateDirectory(settingsFolder);
        string file = Path.Combine(settingsFolder, "ClientAppSettings.json");

        var serializer = new JavaScriptSerializer();
        var settings = ReadClientSettings(file, serializer);

        if (value != null) settings[key] = value;
        else settings.Remove(key);

        File.WriteAllText(file, serializer.Serialize(settings));
        return file;
    }

    static Dictionary<string, object> ReadClientSettings(string file, JavaScriptSerializer serializer)
    {
        if (!File.Exists(file)) return new Dictionary<string, object>();
        string text = File.ReadAllText(file);
        if (string.IsNullOrWhiteSpace(text)) return new Dictionary<string, object>();
        return serializer.Deserialize<Dictionary<string, object>>(text) ?? new Dictionary<string, object>();
    }

    static string FindLatestVersionFolder()
    {
        string versions = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Roblox", "Versions");
        if (!Directory.Exists(versions)) return null;

        return Directory.GetDirectories(versions)
            .Where(d => File.Exists(Path.Combine(d, "RobloxPlayerBeta.exe")))
            .OrderByDescending(d => File.GetLastWriteTimeUtc(Path.Combine(d, "RobloxPlayerBeta.exe")))
            .FirstOrDefault();
    }

    // Every response is disposed. An open response holds a connection, and .NET Framework allows only a
    // few per host, so leaking them makes later requests wait until they time out.
    static string GetCsrfToken(string cookie)
    {
        string token;
        using (HttpWebResponse response = Send("POST", AuthBase + "v1/authentication-ticket/", cookie, null))
        {
            token = response.Headers["x-csrf-token"];
        }
        if (string.IsNullOrEmpty(token))
            throw new InvalidOperationException("No X-CSRF-Token from Roblox. The login cookie may have expired.");
        return token;
    }

    static string GetTicket(string cookie, string csrf)
    {
        using (HttpWebResponse response = Send("POST", AuthBase + "v1/authentication-ticket/", cookie, csrf))
        {
            string ticket = response.Headers["rbx-authentication-ticket"];
            if (string.IsNullOrEmpty(ticket))
            {
                string detail;
                using (var reader = new StreamReader(response.GetResponseStream()))
                {
                    detail = reader.ReadToEnd();
                }
                throw new InvalidOperationException("Roblox did not return an authentication ticket (HTTP " + (int)response.StatusCode + "). " + detail);
            }
            return ticket;
        }
    }

    // A request that times out or loses its connection is tried once more on a fresh connection.
    static HttpWebResponse Send(string method, string url, string cookie, string csrf)
    {
        try
        {
            return SendOnce(method, url, cookie, csrf);
        }
        catch (WebException ex) when (ex.Status == WebExceptionStatus.Timeout || ex.Status == WebExceptionStatus.ConnectionClosed || ex.Status == WebExceptionStatus.KeepAliveFailure)
        {
            try
            {
                return SendOnce(method, url, cookie, csrf);
            }
            catch (WebException again)
            {
                // Name the request that failed, so the error shows where Roblox stopped answering.
                string host = new Uri(url).Host;
                throw new InvalidOperationException(
                    "Roblox did not answer " + method + " " + host + " after 2 tries (" + again.Status + "). " +
                    "If you use a VPN, proxy or firewall, check that it allows Roblox.", again);
            }
        }
    }

    static HttpWebResponse SendOnce(string method, string url, string cookie, string csrf)
    {
        var request = (HttpWebRequest)WebRequest.Create(url);
        request.Method = method;
        request.Referer = Referer;
        request.AllowAutoRedirect = false;
        // The launcher stays running between launches, so a pooled connection can go stale.
        // Each request opens its own connection and gives up after 5 seconds.
        request.KeepAlive = false;
        request.Timeout = 5000;
        request.ReadWriteTimeout = 5000;
        request.UserAgent = "RBX-Alt-Manager-Lite";
        if (cookie != null) request.Headers.Add(HttpRequestHeader.Cookie, ".ROBLOSECURITY=" + cookie);
        if (csrf != null) request.Headers["X-CSRF-TOKEN"] = csrf;

        if (method == "POST")
        {
            // Roblox answers 415 (Unsupported Media Type) to POSTs without a JSON body.
            byte[] body = Encoding.UTF8.GetBytes("{}");
            request.ContentType = "application/json";
            request.ContentLength = body.Length;
            using (Stream stream = request.GetRequestStream())
            {
                stream.Write(body, 0, body.Length);
            }
        }

        try
        {
            return (HttpWebResponse)request.GetResponse();
        }
        catch (WebException ex) when (ex.Response != null)
        {
            return (HttpWebResponse)ex.Response;
        }
    }
}
