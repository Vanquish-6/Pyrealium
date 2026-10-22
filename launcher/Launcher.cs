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

internal sealed class LauncherWindow : Form {
    private readonly TextBox account = new TextBox(), host = new TextBox(), game = new TextBox(), client = new TextBox();
    private readonly NumericUpDown port = new NumericUpDown();
    private readonly Label status = new Label();
    private readonly Button connect = new Button();
    private readonly Color muted = Color.FromArgb(162, 176, 193);
    public LauncherWindow() {
        Text = "Pyrealium • AC2 Launcher"; ClientSize = new Size(790, 700); MinimumSize = new Size(806, 739);
        StartPosition = FormStartPosition.CenterScreen; BackColor = Color.FromArgb(17, 23, 33); ForeColor = Color.White;
        Font = new Font("Segoe UI", 10); AutoScaleMode = AutoScaleMode.Dpi;
        var layout = new TableLayoutPanel { Dock = DockStyle.Fill, Padding = new Padding(30, 24, 30, 22), ColumnCount = 1, RowCount = 11 };
        int[] heights = { 48, 35, 53, 30, 62, 30, 62, 48, 59, 57 };
        foreach (int height in heights) layout.RowStyles.Add(new RowStyle(SizeType.Absolute, height));
        layout.RowStyles.Add(new RowStyle(SizeType.Percent, 100)); Controls.Add(layout);
        layout.Controls.Add(new Label { Text = "PYREALIUM", Dock = DockStyle.Fill, Font = new Font("Segoe UI", 27, FontStyle.Bold), ForeColor = Color.FromArgb(105, 226, 194) });
        layout.Controls.Add(new Label { Text = "Never played it. Already improving the commute.", Dock = DockStyle.Fill, ForeColor = muted });
        var connection = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 3, RowCount = 2 };
        connection.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 35)); connection.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 45)); connection.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 20));
        connection.RowStyles.Add(new RowStyle(SizeType.Absolute, 23)); connection.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        connection.Controls.Add(Caption("Account"), 0, 0); connection.Controls.Add(Caption("Server address"), 1, 0); connection.Controls.Add(Caption("UDP port"), 2, 0);
        account.Dock = host.Dock = port.Dock = DockStyle.Fill; account.MaxLength = 32; host.MaxLength = 253; port.Minimum = 1024; port.Maximum = 65534;
        connection.Controls.Add(account, 0, 1); connection.Controls.Add(host, 1, 1); connection.Controls.Add(port, 2, 1); layout.Controls.Add(connection);
        layout.Controls.Add(Caption("Game assets folder")); layout.Controls.Add(PathRow(game, true));
        layout.Controls.Add(Caption("Recovered client executable • build 7148")); layout.Controls.Add(PathRow(client, false));
        layout.Controls.Add(new Label { Text = "Select compatible game assets and the recovered 7148 executable separately. For world testing, use the prepared compat-7148 folder. Build 7230 is not supported yet.", Dock = DockStyle.Fill, ForeColor = muted });
        connect.Text = "Connect to Pyrealium  →"; connect.Dock = DockStyle.Fill; connect.BackColor = Color.FromArgb(105, 226, 194); connect.ForeColor = Color.FromArgb(17, 23, 33); connect.FlatStyle = FlatStyle.Flat; connect.Font = new Font("Segoe UI", 12, FontStyle.Bold); connect.Cursor = Cursors.Hand; connect.Click += Connect; layout.Controls.Add(connect);
        status.Text = "Ready. Use the account and address supplied by your server operator."; status.Dock = DockStyle.Fill; status.Padding = new Padding(0, 10, 0, 0); status.ForeColor = muted; layout.Controls.Add(status);
        layout.Controls.Add(new Label { Text = "EARLY DEVELOPMENT • Local developer authentication only.\nNo password authentication or playable-world guarantee. The launcher starts the client; it cannot confirm connection.", Dock = DockStyle.Fill, ForeColor = muted, Font = new Font("Segoe UI", 9) });
        var s = Launcher.Load(); account.Text = s.Account; host.Text = s.Host; port.Value = Math.Max(1024, Math.Min(65534, s.Port)); game.Text = s.GameDirectory; client.Text = s.ClientExecutable;
        AcceptButton = connect;
    }
    private Label Caption(string text) { return new Label { Text = text, Dock = DockStyle.Fill, TextAlign = ContentAlignment.BottomLeft, Padding = new Padding(0, 0, 0, 4), ForeColor = muted }; }
    private Control PathRow(TextBox box, bool directory) {
        var row = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 2 };
        row.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100)); row.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 100)); row.RowStyles.Add(new RowStyle(SizeType.Absolute, 31)); row.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        box.Dock = DockStyle.Fill; var browse = new Button { Text = "Browse…", Dock = DockStyle.Fill, BackColor = Color.FromArgb(43, 55, 72), FlatStyle = FlatStyle.Flat };
        browse.Click += delegate { if (directory) { using (var dialog = new FolderBrowserDialog { Description = "Select the AC2 folder containing portal.dat and ijl15.dll", SelectedPath = box.Text, ShowNewFolderButton = false }) if (dialog.ShowDialog(this) == DialogResult.OK) box.Text = dialog.SelectedPath; } else { using (var dialog = new OpenFileDialog { Title = "Select the recovered build 7148 client", Filter = "AC2 client executable (*.exe)|*.exe", CheckFileExists = true }) if (dialog.ShowDialog(this) == DialogResult.OK) box.Text = dialog.FileName; } };
        row.Controls.Add(box, 0, 0); row.Controls.Add(browse, 1, 0);
        var hint = new Label { Text = directory ? "Your existing installation; no game files are downloaded or changed." : "This is usually ac2client.exe in the recovery folder.", Dock = DockStyle.Fill, ForeColor = muted, Font = new Font("Segoe UI", 8) }; row.Controls.Add(hint, 0, 1); row.SetColumnSpan(hint, 2); return row;
    }
    private void Connect(object sender, EventArgs args) {
        connect.Enabled = false;
        try {
            var s = new Settings { Account = account.Text.Trim(), Host = host.Text.Trim(), Port = (int)port.Value, GameDirectory = game.Text.Trim(), ClientExecutable = client.Text.Trim() };
            string error = Launcher.ValidateConnection(s) ?? Launcher.ValidateFiles(s);
            if (error != null) { status.Text = error; MessageBox.Show(this, error, "Check your connection details", MessageBoxButtons.OK, MessageBoxIcon.Information); return; }
            if (Process.GetProcessesByName(Path.GetFileNameWithoutExtension(s.ClientExecutable)).Length > 0) { status.Text = "Close your existing AC2 client before launching another."; return; }
            string saveWarning = "";
            try { Launcher.Save(s); } catch (Exception) { saveWarning = " Settings could not be saved."; }
            using (Process process = Process.Start(Launcher.StartInfo(s))) { }
            status.Text = "Client started. Check the game window for connection progress." + saveWarning;
        } catch (System.ComponentModel.Win32Exception ex) { status.Text = ex.NativeErrorCode == 1223 ? "Launch cancelled. You can try again when ready." : "Could not start the client: " + ex.Message; }
        catch (Exception ex) { status.Text = "Could not start the client: " + ex.Message; }
        finally { connect.Enabled = true; }
    }
}
