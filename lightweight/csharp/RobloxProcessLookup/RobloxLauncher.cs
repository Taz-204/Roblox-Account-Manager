// Account cookie protection (Windows DPAPI) and game launching.
// The launch flow matches the original Account.cs in the C# app:
//   1. POST auth.roblox.com/v1/authentication-ticket/ -> 403 with an X-CSRF-TOKEN header
//   2. POST again with that token -> rbx-authentication-ticket header
//   3. Open a roblox-player: link that carries the ticket and the place launcher URL.
using System;
using System.Diagnostics;
using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Web;

static class RobloxLauncher
{
    const string AuthBase = "https://auth.roblox.com/";
    const string Referer = "https://www.roblox.com/games/4924922222/Brookhaven-RP";

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

    // Starts the game for the account. Throws if the cookie is expired or Roblox refuses.
    public static void Launch(string cookie, long placeId)
    {
        string csrf = GetCsrfToken(cookie);
        string ticket = GetTicket(cookie, csrf);

        Random random = new Random();
        string browserTrackerId = random.Next(100000, 175000).ToString() + random.Next(100000, 900000).ToString();
        long launchTime = (long)Math.Floor((DateTime.UtcNow - new DateTime(1970, 1, 1)).TotalSeconds * 1000);

        string placeLauncher = "https://assetgame.roblox.com/game/PlaceLauncher.ashx?request=RequestGame"
            + "&browserTrackerId=" + browserTrackerId
            + "&placeId=" + placeId
            + "&isPlayTogetherGame=false";

        string link = "roblox-player:1+launchmode:play+gameinfo:" + ticket
            + "+launchtime:" + launchTime
            + "+placelauncherurl:" + HttpUtility.UrlEncode(placeLauncher)
            + "+browsertrackerid:" + browserTrackerId
            + "+robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp";

        Process.Start(new ProcessStartInfo { FileName = link, UseShellExecute = true });
    }

    static string GetCsrfToken(string cookie)
    {
        HttpWebResponse response = Send(AuthBase + "v1/authentication-ticket/", cookie, null);
        string token = response.Headers["x-csrf-token"];
        if (string.IsNullOrEmpty(token))
            throw new InvalidOperationException("No X-CSRF-Token from Roblox. The login cookie may have expired.");
        return token;
    }

    static string GetTicket(string cookie, string csrf)
    {
        HttpWebResponse response = Send(AuthBase + "v1/authentication-ticket/", cookie, csrf);
        string ticket = response.Headers["rbx-authentication-ticket"];
        if (string.IsNullOrEmpty(ticket))
            throw new InvalidOperationException("Roblox did not return an authentication ticket (HTTP " + (int)response.StatusCode + ").");
        return ticket;
    }

    // Roblox answers the first request with 403 on purpose, so error statuses are read, not thrown.
    static HttpWebResponse Send(string url, string cookie, string csrf)
    {
        var request = (HttpWebRequest)WebRequest.Create(url);
        request.Method = "POST";
        request.Referer = Referer;
        request.ContentLength = 0;
        request.AllowAutoRedirect = false;
        request.UserAgent = "RBX-Alt-Manager-Lite";
        request.Headers.Add(HttpRequestHeader.Cookie, ".ROBLOSECURITY=" + cookie);
        if (csrf != null) request.Headers["X-CSRF-TOKEN"] = csrf;

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
