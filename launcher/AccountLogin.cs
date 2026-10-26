// SPDX-License-Identifier: AGPL-3.0-only
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Net;
using System.Security.Principal;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Web.Script.Serialization;
using Microsoft.Win32;

internal static class AccountLogin {
    private const string RegistryPath = @"Software\Turbine\ac2";
    internal static bool IsLoopback(string host) {
        IPAddress address;
        return String.Equals(host, "localhost", StringComparison.OrdinalIgnoreCase) ||
            (IPAddress.TryParse(host, out address) && address.AddressFamily == System.Net.Sockets.AddressFamily.InterNetwork && IPAddress.IsLoopback(address));
    }
    private static Uri Endpoint(string value, string action) {
        Uri uri;
        if (!Uri.TryCreate(value, UriKind.Absolute, out uri) || uri.UserInfo.Length != 0 || uri.Query.Length != 0 || uri.Fragment.Length != 0 || uri.AbsolutePath != "/" ||
            (uri.Scheme != "https" && !(uri.Scheme == "http" && IsLoopback(uri.Host))))
            throw new ArgumentException("Use an HTTPS account-service address. HTTP is allowed only on this PC.");
        // Use IPv4 for the local backend and source-bound native ticket.
        if (uri.Host.Equals("localhost", StringComparison.OrdinalIgnoreCase)) {
            var builder = new UriBuilder(uri); builder.Host = "127.0.0.1"; uri = builder.Uri;
        }
        return new Uri(uri, "api/" + action);
    }
    internal static void Register(Settings settings, string password) {
        if (password.Length < 12 || password.Length > 128) throw new ArgumentException("Choose a password of 12-128 characters.");
        Request(settings, password, "register");
    }
    private static Dictionary<string, object> Request(Settings settings, string password, string action) {
        var json = new JavaScriptSerializer { MaxJsonLength = 8192 };
        byte[] body = Encoding.UTF8.GetBytes(json.Serialize(new { account = settings.Account.ToLowerInvariant(), password = password }));
        try {
            ServicePointManager.SecurityProtocol |= SecurityProtocolType.Tls12;
            var request = (HttpWebRequest)WebRequest.Create(Endpoint(settings.AccountsUrl, action));
            request.Method = "POST"; request.ContentType = "application/json; charset=utf-8";
            request.Timeout = 15000; request.ReadWriteTimeout = 15000; request.AllowAutoRedirect = false;
            request.ContentLength = body.Length;
            using (var output = request.GetRequestStream()) output.Write(body, 0, body.Length);
            using (var response = (HttpWebResponse)request.GetResponse()) {
                if ((int)response.StatusCode != (action == "register" ? 201 : 200)) throw new InvalidOperationException("The account service returned an unexpected response.");
                using (var input = new StreamReader(response.GetResponseStream())) {
                    var chars = new char[8193]; int length = 0, count;
                    while (length < chars.Length && (count = input.Read(chars, length, chars.Length - length)) > 0) length += count;
                    if (length > 8192) throw new InvalidOperationException("The account service response was too large.");
                    return json.Deserialize<Dictionary<string, object>>(new string(chars, 0, length));
                }
            }
        } catch (WebException error) {
            var response = error.Response as HttpWebResponse;
            if (response != null) using (response) {
                int code = (int)response.StatusCode;
                if (code == 401) throw new InvalidOperationException("Account or password is incorrect.");
                if (code == 409) throw new InvalidOperationException("That account name is already in use or reserved.");
                if (code == 429) throw new InvalidOperationException("Too many attempts. Wait a minute and try again.");
                if (code == 400) throw new InvalidOperationException("Use an available account name and a password of 12-128 characters.");
                if (code == 503) throw new InvalidOperationException("The account service could not complete the request. Please try again; if it continues, its server log needs checking.");
                throw new InvalidOperationException("The account service rejected the request (HTTP " + code + "). Check its address in Settings.");
            }
            throw new InvalidOperationException("Could not reach the account service. Check its address in Settings and that it is running.");
        } finally { Array.Clear(body, 0, body.Length); }
    }
    internal static void Launch(Settings settings, string password) {
        if (password.Length == 0 && settings.Account == "local" && IsLoopback(settings.Host)) {
            using (Process process = Process.Start(Launcher.StartInfo(settings))) { }
            return;
        }
        if (password.Length == 0) throw new ArgumentException("Enter your password, or use Create account for a new identity.");
        string identity = WindowsIdentity.GetCurrent().User.Value;
        using (var gate = new Mutex(false, @"Local\PyrealiumTicket_" + identity)) {
            bool held = false; string ticket = null;
            try {
                try { held = gate.WaitOne(0); } catch (AbandonedMutexException) { held = true; }
                if (!held) throw new InvalidOperationException("Another account launch is already in progress.");
                var login = Request(settings, password, "login");
                string name = Convert.ToString(login["account"]), token = Convert.ToString(login["ticket"]);
                var target = new Settings { Account = name, Host = Convert.ToString(login["game_host"]), Port = Convert.ToInt32(login["game_port"]), GameDirectory = settings.GameDirectory, ClientExecutable = settings.ClientExecutable };
                if (name != settings.Account.ToLowerInvariant() || !Regex.IsMatch(token, @"\A[a-f0-9]{64}\z") || Launcher.ValidateConnection(target) != null)
                    throw new InvalidOperationException("The account service returned invalid connection details.");
                ticket = token;
                using (var key = Registry.CurrentUser.CreateSubKey(RegistryPath)) key.SetValue("GLSTicket", ticket, RegistryValueKind.String);
                var start = Launcher.StartInfo(target); start.Arguments += " /glsticket";
                using (Process process = Process.Start(start)) {
                    DateTime deadline = DateTime.UtcNow.AddSeconds(45);
                    while (process != null && !process.HasExited && DateTime.UtcNow < deadline) {
                        using (var key = Registry.CurrentUser.OpenSubKey(RegistryPath))
                            if (key == null || key.GetValue("GLSTicket") == null) return;
                        Thread.Sleep(200); process.Refresh();
                    }
                    throw new InvalidOperationException("The client did not consume its sign-in ticket. Close the client and try again.");
                }
            } finally {
                if (ticket != null) using (var key = Registry.CurrentUser.OpenSubKey(RegistryPath, true)) {
                    if (key != null && String.Equals(key.GetValue("GLSTicket") as string, ticket, StringComparison.Ordinal)) key.DeleteValue("GLSTicket", false);
                }
                if (held) gate.ReleaseMutex();
            }
        }
    }
}
