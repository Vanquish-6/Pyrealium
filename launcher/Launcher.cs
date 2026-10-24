using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Text.RegularExpressions;
using System.Windows.Forms;
using System.Xml.Serialization;

[Serializable]
public sealed class Settings {
    public string Account = "local";
    public string Host = "127.0.0.1";
    public int Port = 19070;
    public string GameDirectory = "";
    public string ClientExecutable = "";
}

internal static class Launcher {
    internal static readonly string SettingsPath = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Pyrealium", "launcher.xml");
    internal static string ValidateConnection(Settings s) {
        if (!Regex.IsMatch(s.Account ?? "", @"\A[A-Za-z0-9_]{1,32}\z")) return "Account must contain 1–32 letters, numbers, or underscores.";
        string host = s.Host ?? "";
        if (host.Length == 0 || host.Length > 253) return "Enter an IPv4 address or DNS hostname (without a URL or port).";
        foreach (string label in host.Split('.'))
            if (!Regex.IsMatch(label, @"\A[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\z")) return "Enter an IPv4 address or DNS hostname (without spaces, a URL, or a port).";
        if (Regex.IsMatch(host, @"\A[0-9.]+\z")) {
            string[] parts = host.Split('.');
            if (parts.Length != 4) return "The IPv4 address must have four numbers from 0 to 255.";
            foreach (string part in parts) { byte value; if (!Byte.TryParse(part, out value)) return "The IPv4 address must have four numbers from 0 to 255."; }
        }
        if (s.Port < 1024 || s.Port > 65534) return "Port must be between 1024 and 65534; Pyrealium also uses the next port.";
        return null;
    }
    internal static string ValidateFiles(Settings s) {
        if (!Directory.Exists(s.GameDirectory)) return "Choose your installed AC2 game folder.";
        foreach (string file in new [] { "portal.dat", "ijl15.dll" }) {
            string path = Path.Combine(s.GameDirectory, file);
            if (!File.Exists(path) || new FileInfo(path).Length == 0) return "The game folder needs a nonempty " + file + ".";
        }
        if (!File.Exists(s.ClientExecutable) || !String.Equals(Path.GetExtension(s.ClientExecutable), ".exe", StringComparison.OrdinalIgnoreCase)) return "Choose the recovered build 7148 client executable.";
        string version = FileVersionInfo.GetVersionInfo(s.ClientExecutable).FileVersion ?? "";
        if (!Regex.IsMatch(version, @"(?:^|\.)7148\z")) return "This launcher requires client build 7148. Selected version: " + (version.Length == 0 ? "unknown" : version) + ". The installed 7230 executable is a different build.";
        return null;
    }
    internal static ProcessStartInfo StartInfo(Settings s) {
        // Inputs are restricted to single ASCII tokens before reaching this function.
        string error = ValidateConnection(s);
        if (error != null) throw new ArgumentException(error);
        return new ProcessStartInfo {
            FileName = Path.GetFullPath(s.ClientExecutable), WorkingDirectory = Path.GetFullPath(s.GameDirectory),
            Arguments = "--account " + s.Account + " --host " + s.Host + " --port " + s.Port.ToString(System.Globalization.CultureInfo.InvariantCulture) + " --rodat --width 1024 --height 768",
            UseShellExecute = true
        };
    }
    internal static Settings Load() {
        try { using (var input = File.OpenRead(SettingsPath)) return (Settings)new XmlSerializer(typeof(Settings)).Deserialize(input); }
        catch { return new Settings(); }
    }
    internal static void Save(Settings s) {
        Directory.CreateDirectory(Path.GetDirectoryName(SettingsPath));
        string temporary = SettingsPath + ".tmp";
        using (var output = File.Create(temporary)) new XmlSerializer(typeof(Settings)).Serialize(output, s);
        if (File.Exists(SettingsPath)) File.Replace(temporary, SettingsPath, null); else File.Move(temporary, SettingsPath);
    }
    [STAThread]
    public static int Main(string[] args) {
        if (args.Length > 0 && args[0] == "--self-test") return SelfTest(args.Length > 1 ? args[1] : null);
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        if (args.Length > 1 && args[0] == "--preview") {
            using (var window = new LauncherWindow()) {
                if (args.Length > 2 && args[2] == "settings") window.ShowSettings(true);
                window.Show(); Application.DoEvents();
                using (var bitmap = new Bitmap(window.Width, window.Height)) {
                    window.DrawToBitmap(bitmap, new Rectangle(Point.Empty, window.Size));
                    bitmap.Save(Path.GetFullPath(args[1]), System.Drawing.Imaging.ImageFormat.Png);
                }
                window.Close();
            }
            return 0;
        }
        Application.Run(new LauncherWindow());
        return 0;
    }
    private static int SelfTest(string report) {
        int checks = 0;
        Action<bool> check = delegate(bool ok) { checks++; if (!ok) throw new Exception("Check " + checks + " failed"); };
        try {
            var s = new Settings { GameDirectory = @"C:\Game folder's assets", ClientExecutable = @"C:\Recovered client\ac2client.exe" };
            check(ValidateConnection(s) == null);
            foreach (string value in new [] { "", "a b", "x\" --host attacker", "a;whoami", new string('a', 33) }) { s.Account = value; check(ValidateConnection(s) != null); }
            s.Account = "Player_17";
            foreach (string value in new [] { "https://example.com", "a b", "-flag", "a\nb", "256.1.1.1", "1.2.3", "example..com" }) { s.Host = value; check(ValidateConnection(s) != null); }
            foreach (string value in new [] { "127.0.0.1", "localhost", "play.example.com" }) { s.Host = value; check(ValidateConnection(s) == null); }
            s.Port = 65535; check(ValidateConnection(s) != null); s.Port = 1023; check(ValidateConnection(s) != null); s.Port = 19070;
            var info = StartInfo(s);
            check(info.UseShellExecute && info.FileName == s.ClientExecutable && info.WorkingDirectory == s.GameDirectory);
            check(info.Arguments == "--account Player_17 --host play.example.com --port 19070 --rodat --width 1024 --height 768");
            check(ValidateFiles(s) != null);
            if (report != null) File.WriteAllText(report, checks + " launcher checks passed. No game process was started.\r\n");
            return 0;
        } catch (Exception ex) { if (report != null) File.WriteAllText(report, ex.ToString()); return 1; }
    }
}
