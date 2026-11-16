// Account cookie protection, username lookup, FPS cap, and game launching.
// The launch flow matches the original Account.cs in the C# app:
//   1. POST auth.roblox.com/v1/authentication-ticket/ -> 403 with an X-CSRF-TOKEN header
//   2. POST again with that token -> rbx-authentication-ticket header
//   3. Open a roblox-player: link that carries the ticket and the place launcher URL.
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
    const string Referer = "https://www.roblox.com/games/4924922222/Brookhaven-RP";
    const string FpsKey = "DFIntTaskSchedulerTargetFps";

    static RobloxLauncher()
    {
        ServicePointManager.SecurityProtocol |= SecurityProtocolType.Tls12;
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

    // Returns the encrypted cookie and the account's username ("" if Roblox didn't say).
    public static string[] ProtectWithUsername(string cookie)
    {
        string blob = Protect(cookie);
        string username = GetUsername(cookie) ?? "";
        return new[] { blob, username };
    }

    public static string GetUsername(string cookie)
    {
        HttpWebResponse response = Send("GET", UsersBase + "v1/users/authenticated", cookie, null);
        if ((int)response.StatusCode != 200) return null;

        using (var reader = new StreamReader(response.GetResponseStream()))
        {
            var data = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(reader.ReadToEnd());
            object name;
            return data != null && data.TryGetValue("name", out name) ? name as string : null;
        }
    }

    // Starts the game for the account. An empty jobId joins any server; a job ID joins that server.
    public static void Launch(string cookie, long placeId, string jobId)
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

        string link = "roblox-player:1+launchmode:play+gameinfo:" + ticket
            + "+launchtime:" + launchTime
            + "+placelauncherurl:" + HttpUtility.UrlEncode(placeLauncher)
            + "+browsertrackerid:" + browserTrackerId
            + "+robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp";

        Process.Start(new ProcessStartInfo { FileName = link, UseShellExecute = true });
    }

    // Sets the FPS cap in Roblox's client settings. 0 removes the cap. Returns the file written.
    // This is a client-wide setting, so it applies to every Roblox instance. Roblox must restart to pick it up.
    public static string SetFps(int fps)
    {
        string versionFolder = FindLatestVersionFolder();
        if (versionFolder == null)
            throw new InvalidOperationException("Roblox is not installed yet. Start Roblox once, then try again.");

        string settingsFolder = Path.Combine(versionFolder, "ClientSettings");
        Directory.CreateDirectory(settingsFolder);
        string file = Path.Combine(settingsFolder, "ClientAppSettings.json");

        var serializer = new JavaScriptSerializer();
        var settings = new Dictionary<string, object>();
        if (File.Exists(file))
        {
            string text = File.ReadAllText(file);
            if (!string.IsNullOrWhiteSpace(text))
                settings = serializer.Deserialize<Dictionary<string, object>>(text) ?? settings;
        }

        if (fps > 0) settings[FpsKey] = fps;
        else settings.Remove(FpsKey);

        File.WriteAllText(file, serializer.Serialize(settings));
        return file;
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

    static string GetCsrfToken(string cookie)
    {
        HttpWebResponse response = Send("POST", AuthBase + "v1/authentication-ticket/", cookie, null);
        string token = response.Headers["x-csrf-token"];
        if (string.IsNullOrEmpty(token))
            throw new InvalidOperationException("No X-CSRF-Token from Roblox. The login cookie may have expired.");
        return token;
    }

    static string GetTicket(string cookie, string csrf)
    {
        HttpWebResponse response = Send("POST", AuthBase + "v1/authentication-ticket/", cookie, csrf);
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

    // Roblox answers the first request with 403 on purpose, so error statuses are read, not thrown.
    static HttpWebResponse Send(string method, string url, string cookie, string csrf)
    {
        var request = (HttpWebRequest)WebRequest.Create(url);
        request.Method = method;
        request.Referer = Referer;
        request.AllowAutoRedirect = false;
        request.UserAgent = "RBX-Alt-Manager-Lite";
        request.Headers.Add(HttpRequestHeader.Cookie, ".ROBLOSECURITY=" + cookie);
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
