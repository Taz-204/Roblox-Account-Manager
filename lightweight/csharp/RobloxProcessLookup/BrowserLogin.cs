// Opens a Chromium-based window (WebView2) at roblox.com/login. The user signs in there.
// Once Roblox sets the .ROBLOSECURITY cookie, the window closes and the cookie is returned.
// The cookie is read from the window's own browser session. Nothing is read from the
// user's regular Chrome or Edge profile.
using System;
using System.Windows.Forms;
using Microsoft.Web.WebView2.WinForms;

static class BrowserLogin
{
    // Returns the cookie value, or null if the window was closed before signing in.
    public static string Run()
    {
        string cookie = null;
        Exception error = null;

        var form = new Form
        {
            Text = "Log in to Roblox",
            Width = 1000,
            Height = 760,
            StartPosition = FormStartPosition.CenterScreen
        };
        var web = new WebView2 { Dock = DockStyle.Fill };
        form.Controls.Add(web);

        var timer = new System.Windows.Forms.Timer { Interval = 1500 };
        bool checking = false;

        timer.Tick += async (sender, args) =>
        {
            if (checking || web.CoreWebView2 == null) return;
            checking = true;
            try
            {
                var cookies = await web.CoreWebView2.CookieManager.GetCookiesAsync("https://www.roblox.com");
                foreach (var c in cookies)
                {
                    if (c.Name == ".ROBLOSECURITY" && !string.IsNullOrEmpty(c.Value))
                    {
                        cookie = c.Value;
                        timer.Stop();
                        form.Close();
                        break;
                    }
                }
            }
            catch (Exception ex)
            {
                error = ex;
                timer.Stop();
                form.Close();
            }
            finally
            {
                checking = false;
            }
        };

        form.Shown += async (sender, args) =>
        {
            try
            {
                await web.EnsureCoreWebView2Async();
                web.CoreWebView2.Navigate("https://www.roblox.com/login");
                timer.Start();
            }
            catch (Exception ex)
            {
                error = ex;
                form.Close();
            }
        };

        Application.EnableVisualStyles();
        Application.Run(form);
        timer.Dispose();

        if (error != null) throw new InvalidOperationException("Could not open the login window: " + error.Message, error);
        return cookie;
    }
}
