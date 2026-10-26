using System;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.IO;
using System.Reflection;
using System.Windows.Forms;
using System.Threading.Tasks;

internal static class Theme {
    internal static readonly Color Ink = Color.FromArgb(20, 16, 20), Panel = Color.FromArgb(32, 25, 29), Field = Color.FromArgb(48, 38, 41), Muted = Color.FromArgb(183, 169, 159), Accent = Color.FromArgb(197, 216, 117), Gold = Color.FromArgb(224, 187, 125), Parchment = Color.FromArgb(248, 237, 215);
    internal static Image Asset(string name) {
        using (Stream stream = Assembly.GetExecutingAssembly().GetManifestResourceStream("Pyrealium." + name)) {
            if (stream == null) throw new InvalidOperationException("Missing launcher artwork: " + name);
            using (Image original = Image.FromStream(stream)) return new Bitmap(original);
        }
    }
    internal static void Cover(Graphics g, Image image, Rectangle bounds) {
        float scale = Math.Max((float)bounds.Width / image.Width, (float)bounds.Height / image.Height);
        float w = image.Width * scale, h = image.Height * scale;
        g.DrawImage(image, bounds.X + (bounds.Width - w) / 2, bounds.Y + (bounds.Height - h) / 2, w, h);
    }
    internal static Label Text(string value, float size, Color color, FontStyle style) {
        return new Label { Text = value, AutoSize = false, Dock = DockStyle.Fill, ForeColor = color, BackColor = Color.Transparent, Font = new Font("Segoe UI", size, style), TextAlign = ContentAlignment.MiddleLeft, Margin = Padding.Empty };
    }
    internal static Button Button(string value, bool primary) {
        var button = new Button { Text = value, Dock = DockStyle.Fill, FlatStyle = FlatStyle.Flat, BackColor = primary ? Accent : Field, ForeColor = primary ? Ink : Theme.Parchment, Font = new Font("Segoe UI", 10, FontStyle.Bold), Cursor = Cursors.Hand, Margin = new Padding(0, 0, 0, 0), UseVisualStyleBackColor = false };
        button.FlatAppearance.BorderSize = 0;
        button.FlatAppearance.MouseOverBackColor = primary ? Color.FromArgb(222, 236, 156) : Color.FromArgb(66, 49, 48);
        button.FlatAppearance.MouseDownBackColor = primary ? Color.FromArgb(166, 188, 90) : Color.FromArgb(79, 58, 53);
        return button;
    }
}

internal sealed class ArtworkPanel : Panel {
    private readonly Image artwork;
    private readonly bool hero;
    internal ArtworkPanel(string asset, bool isHero) { artwork = Theme.Asset(asset); hero = isHero; DoubleBuffered = true; }
    protected override void OnPaintBackground(PaintEventArgs e) {
        Theme.Cover(e.Graphics, artwork, ClientRectangle);
        if (Width < 1 || Height < 1) return;
        using (var shade = new LinearGradientBrush(ClientRectangle, Color.FromArgb(hero ? 185 : 25, Theme.Ink), Color.FromArgb(hero ? 10 : 240, Theme.Ink), hero ? 0f : 90f)) e.Graphics.FillRectangle(shade, ClientRectangle);
        if (hero) using (var shade = new LinearGradientBrush(ClientRectangle, Color.FromArgb(0, Theme.Ink), Color.FromArgb(240, Theme.Ink), 90f)) e.Graphics.FillRectangle(shade, ClientRectangle);
    }
    protected override void Dispose(bool disposing) { if (disposing) artwork.Dispose(); base.Dispose(disposing); }
}

internal sealed class LauncherWindow : Form {
    private readonly TextBox account = new TextBox(), host = new TextBox(), game = new TextBox(), client = new TextBox();
    private readonly TextBox password = new TextBox(), accountsUrl = new TextBox();
    private readonly Button register = Theme.Button("CREATE ACCOUNT", false);
    private readonly NumericUpDown port = new NumericUpDown();
    private readonly Label status = Theme.Text("Ready when you are.", 9, Theme.Muted, FontStyle.Regular);
    private readonly Button connect = Theme.Button("ENTER DERETH   \u2192", true);
    private readonly Button playTab = Theme.Button("PLAY", false), settingsTab = Theme.Button("SETTINGS", false);
    private readonly Panel pages = new Panel { Dock = DockStyle.Fill, BackColor = Theme.Panel };
    private TableLayoutPanel playPage, settingsPage;
    private bool settingsVisible;

    internal LauncherWindow() {
        Text = "Pyrealium | Asheron's Call 2: Legions"; ClientSize = new Size(1100, 790); MinimumSize = new Size(1000, 790);
        StartPosition = FormStartPosition.CenterScreen; BackColor = Theme.Ink; ForeColor = Theme.Parchment;
        Font = new Font("Segoe UI", 10); AutoScaleDimensions = new SizeF(96, 96); AutoScaleMode = AutoScaleMode.Dpi; FormBorderStyle = FormBorderStyle.None;
        var root = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, RowCount = 3, Margin = Padding.Empty };
        root.RowStyles.Add(new RowStyle(SizeType.Absolute, 66)); root.RowStyles.Add(new RowStyle(SizeType.Percent, 100)); root.RowStyles.Add(new RowStyle(SizeType.Absolute, 48)); Controls.Add(root);
        root.Controls.Add(Header(), 0, 0);
        var body = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1, Margin = Padding.Empty };
        body.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        body.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 60)); body.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 40)); root.Controls.Add(body, 0, 1);
        body.Controls.Add(Hero(), 0, 0);
        var rail = new TableLayoutPanel { Dock = DockStyle.Fill, BackColor = Theme.Panel, Padding = new Padding(30, 23, 30, 22), ColumnCount = 1, RowCount = 4, Margin = Padding.Empty };
        rail.RowStyles.Add(new RowStyle(SizeType.Absolute, 39)); rail.RowStyles.Add(new RowStyle(SizeType.Absolute, 24)); rail.RowStyles.Add(new RowStyle(SizeType.Percent, 100)); rail.RowStyles.Add(new RowStyle(SizeType.Absolute, 78)); body.Controls.Add(rail, 1, 0);
        var tabs = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1, Margin = Padding.Empty };
        tabs.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        tabs.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50)); tabs.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50)); tabs.Controls.Add(playTab, 0, 0); tabs.Controls.Add(settingsTab, 1, 0); rail.Controls.Add(tabs, 0, 0);
        playTab.Click += delegate { ShowSettings(false); }; settingsTab.Click += delegate { ShowSettings(true); };
        rail.Controls.Add(pages, 0, 2); playPage = PlayPage(); settingsPage = SettingsPage(); pages.Controls.Add(playPage); pages.Controls.Add(settingsPage);
        status.TextAlign = ContentAlignment.TopLeft; status.Padding = new Padding(0, 14, 0, 0); status.AccessibleName = "Launch status"; rail.Controls.Add(status, 0, 3);
        root.Controls.Add(Footer(), 0, 2);
        var s = Launcher.Load(); account.Text = s.Account; host.Text = s.Host; port.Value = Math.Max(1024, Math.Min(65534, s.Port)); game.Text = s.GameDirectory; client.Text = s.ClientExecutable; accountsUrl.Text = s.AccountsUrl;
        ShowSettings(false); AcceptButton = connect;
    }
    private Control Header() {
        var bar = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 5, RowCount = 1, Padding = new Padding(28, 10, 12, 10), Margin = Padding.Empty };
        bar.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        bar.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 220)); bar.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100)); bar.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 155)); bar.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 42)); bar.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 42));
        var brand = Theme.Text("\u25c7  P Y R E A L I U M", 13, Theme.Gold, FontStyle.Bold); bar.Controls.Add(brand, 0, 0);
        var edition = Theme.Text("ASHERON'S CALL 2  /  LEGIONS", 9, Theme.Muted, FontStyle.Regular); bar.Controls.Add(edition, 1, 0);
        bar.Controls.Add(Theme.Text("DEVELOPMENT BUILD", 8, Theme.Gold, FontStyle.Bold), 2, 0);
        var minimize = Theme.Button("\u2014", false); minimize.AccessibleName = "Minimize launcher"; minimize.Click += delegate { WindowState = FormWindowState.Minimized; }; bar.Controls.Add(minimize, 3, 0);
        var close = Theme.Button("\u00d7", false); close.AccessibleName = "Close launcher"; close.Click += delegate { Close(); }; bar.Controls.Add(close, 4, 0);
        Point origin = Point.Empty; bool dragging = false;
        foreach (Control surface in new Control[] { bar, brand, edition }) {
            surface.MouseDown += delegate(object sender, MouseEventArgs e) { if (e.Button == MouseButtons.Left) { origin = Cursor.Position; dragging = true; ((Control)sender).Capture = true; } };
            surface.MouseMove += delegate { if (dragging) { Point now = Cursor.Position; Location = new Point(Left + now.X - origin.X, Top + now.Y - origin.Y); origin = now; } };
            surface.MouseUp += delegate(object sender, MouseEventArgs e) { dragging = false; ((Control)sender).Capture = false; };
        }
        return bar;
    }
    private Control Hero() {
        var hero = new ArtworkPanel("legions-portal.png", true) { Dock = DockStyle.Fill, Margin = Padding.Empty };
        var stack = new TableLayoutPanel { Dock = DockStyle.Fill, BackColor = Color.Transparent, Padding = new Padding(36, 30, 30, 26), ColumnCount = 1, RowCount = 8 };
        int[] heights = { 25, 32, 125, 60 }; foreach (int h in heights) stack.RowStyles.Add(new RowStyle(SizeType.Absolute, h));
        stack.RowStyles.Add(new RowStyle(SizeType.Percent, 100)); stack.RowStyles.Add(new RowStyle(SizeType.Absolute, 24)); stack.RowStyles.Add(new RowStyle(SizeType.Absolute, 134)); stack.RowStyles.Add(new RowStyle(SizeType.Absolute, 26)); hero.Controls.Add(stack);
        stack.Controls.Add(Theme.Text("A S H E R O N ' S  C A L L  2", 10, Theme.Gold, FontStyle.Bold), 0, 0);
        var expansion = Theme.Text("L E G I O N S", 18, Theme.Gold, FontStyle.Bold); expansion.Font = new Font("Cambria", 18, FontStyle.Bold); stack.Controls.Add(expansion, 0, 1);
        var title = Theme.Text("A new age.\nAn ancient world.", 37, Theme.Parchment, FontStyle.Bold); title.Font = new Font("Cambria", 37, FontStyle.Bold); title.TextAlign = ContentAlignment.MiddleLeft; stack.Controls.Add(title, 0, 2);
        stack.Controls.Add(Theme.Text("Beyond the ruins, Dereth awaits.\nStep through the portal. Begin again.", 11, Color.FromArgb(225, 209, 188), FontStyle.Regular), 0, 3);
        stack.Controls.Add(Theme.Text("R E D I S C O V E R  D E R E T H", 8, Theme.Muted, FontStyle.Bold), 0, 5);
        var cards = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1, BackColor = Color.Transparent, Margin = Padding.Empty };
        cards.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        cards.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 64)); cards.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 36));
        cards.Controls.Add(ArtCard("dereth.png", "A world worth returning to", "Adventure is still out there."), 0, 0);
        cards.Controls.Add(LegionsCard(), 1, 0); stack.Controls.Add(cards, 0, 6);
        stack.Controls.Add(Theme.Text("INDEPENDENT PROJECT  /  BUILT WITH UNREASONABLE AMBITION", 7, Theme.Muted, FontStyle.Regular), 0, 7);
        return hero;
    }
    private Control ArtCard(string asset, string title, string subtitle) {
        var card = new ArtworkPanel(asset, false) { Dock = DockStyle.Fill, Margin = new Padding(0, 4, 12, 0), Padding = new Padding(12, 0, 8, 10) };
        var text = new TableLayoutPanel { Dock = DockStyle.Bottom, Height = 49, ColumnCount = 1, RowCount = 2, BackColor = Color.Transparent };
        text.RowStyles.Add(new RowStyle(SizeType.Absolute, 25)); text.RowStyles.Add(new RowStyle(SizeType.Absolute, 24));
        text.Controls.Add(Theme.Text(title, 10, Theme.Parchment, FontStyle.Bold), 0, 0); text.Controls.Add(Theme.Text(subtitle, 8, Color.FromArgb(225, 209, 188), FontStyle.Regular), 0, 1); card.Controls.Add(text); return card;
    }
    private Control LegionsCard() {
        var card = new TableLayoutPanel { Dock = DockStyle.Fill, BackColor = Theme.Panel, ColumnCount = 2, RowCount = 1, Margin = new Padding(0, 4, 12, 0) };
        card.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 46)); card.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 54)); card.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        var cover = new PictureBox { Image = Theme.Asset("legions-cover.png"), SizeMode = PictureBoxSizeMode.Zoom, Dock = DockStyle.Fill, Margin = Padding.Empty, AccessibleName = "Asheron's Call 2 Legions cover" };
        cover.Disposed += delegate { cover.Image.Dispose(); }; card.Controls.Add(cover, 0, 0);
        var copy = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, RowCount = 2, Padding = new Padding(8, 27, 3, 23), Margin = Padding.Empty };
        copy.RowStyles.Add(new RowStyle(SizeType.Percent, 40)); copy.RowStyles.Add(new RowStyle(SizeType.Percent, 60));
        var title = Theme.Text("Legions", 12, Theme.Gold, FontStyle.Bold); title.Font = new Font("Cambria", 12, FontStyle.Bold); copy.Controls.Add(title, 0, 0);
        copy.Controls.Add(Theme.Text("A new age\nof Dereth.", 8, Theme.Muted, FontStyle.Regular), 0, 1); card.Controls.Add(copy, 1, 0); return card;
    }
    private TableLayoutPanel Page() { return new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, Margin = Padding.Empty, BackColor = Theme.Panel }; }
    private TableLayoutPanel PlayPage() {
        var page = Page(); page.RowCount = 13;
        int[] rows = { 38, 27, 24, 43, 24, 43, 24, 43, 32, 47, 36, 35 };
        foreach (int h in rows) page.RowStyles.Add(new RowStyle(SizeType.Absolute, h)); page.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        page.Controls.Add(Theme.Text("Your next adventure", 20, Theme.Parchment, FontStyle.Bold), 0, 0);
        page.Controls.Add(Theme.Text("Choose your identity. Cross the threshold.", 9, Theme.Muted, FontStyle.Regular), 0, 1);
        page.Controls.Add(Caption("ACCOUNT NAME"), 0, 2); StyleInput(account); account.AccessibleName = "Account name"; account.MaxLength = 32; page.Controls.Add(Input(account), 0, 3);
        var serverCaptions = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1, Margin = Padding.Empty };
        serverCaptions.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100)); serverCaptions.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 94)); serverCaptions.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        serverCaptions.Controls.Add(Caption("SERVER ADDRESS"), 0, 0); serverCaptions.Controls.Add(Caption("PORT"), 1, 0); page.Controls.Add(serverCaptions, 0, 6);
        var server = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1, Margin = Padding.Empty };
        server.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        server.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100)); server.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 94)); StyleInput(host); host.AccessibleName = "Server address"; host.MaxLength = 253;
        port.Minimum = 1024; port.Maximum = 65534; port.BackColor = Theme.Field; port.ForeColor = Theme.Parchment; port.BorderStyle = BorderStyle.None; port.Font = new Font("Segoe UI", 11); port.Dock = DockStyle.Fill; port.AccessibleName = "Server port";
        var hostFrame = Input(host); hostFrame.Margin = new Padding(0, 0, 8, 5); server.Controls.Add(hostFrame, 0, 0); server.Controls.Add(Input(port), 1, 0); page.Controls.Add(server, 0, 7);
        page.Controls.Add(Caption("PASSWORD"), 0, 4); StyleInput(password); password.UseSystemPasswordChar = true; password.MaxLength = 128; password.AccessibleName = "Password"; page.Controls.Add(Input(password), 0, 5);
        page.Controls.Add(Theme.Text("Existing local profile: leave password blank.", 8, Theme.Muted, FontStyle.Regular), 0, 8);
        connect.Font = new Font("Segoe UI", 12, FontStyle.Bold); connect.Click += Connect; page.Controls.Add(connect, 0, 9);
        register.Click += Register; register.Margin = new Padding(0, 4, 0, 0); page.Controls.Add(register, 0, 10);
        var setup = Theme.Button("Game installation settings  \u2197", false); setup.Font = new Font("Segoe UI", 9); setup.BackColor = Theme.Panel; setup.Click += delegate { ShowSettings(true); }; page.Controls.Add(setup, 0, 11);
        return page;
    }
    private TableLayoutPanel SettingsPage() {
        var page = Page(); page.RowCount = 12; int[] rows = { 40, 49, 24, 43, 31, 24, 43, 49, 24, 43, 43 };
        foreach (int h in rows) page.RowStyles.Add(new RowStyle(SizeType.Absolute, h)); page.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        page.Controls.Add(Theme.Text("Prepare your journey", 19, Theme.Parchment, FontStyle.Bold), 0, 0);
        page.Controls.Add(Theme.Text("Select your game assets and recovered client.\nYour choices are saved when you launch.", 9, Theme.Muted, FontStyle.Regular), 0, 1);
        page.Controls.Add(Caption("GAME ASSETS FOLDER"), 0, 2); page.Controls.Add(PathRow(game, true), 0, 3);
        page.Controls.Add(Theme.Text("Contains portal.dat and ijl15.dll.", 8, Theme.Muted, FontStyle.Regular), 0, 4);
        page.Controls.Add(Caption("CLIENT EXECUTABLE  /  BUILD 7148"), 0, 5); page.Controls.Add(PathRow(client, false), 0, 6);
        page.Controls.Add(Theme.Text("Use the prepared compat-7148 assets for world\ntesting. Client build 7230 is not supported.", 9, Theme.Muted, FontStyle.Regular), 0, 7);
        page.Controls.Add(Caption("ACCOUNT SERVICE URL"), 0, 8); StyleInput(accountsUrl); accountsUrl.AccessibleName = "Account service URL"; accountsUrl.MaxLength = 2048; page.Controls.Add(Input(accountsUrl), 0, 9);
        var back = Theme.Button("BACK TO PLAY   \u2192", true); back.Click += delegate { ShowSettings(false); }; page.Controls.Add(back, 0, 10); return page;
    }
    private Control Footer() {
        var footer = new TableLayoutPanel { Dock = DockStyle.Fill, Padding = new Padding(28, 0, 28, 0), ColumnCount = 2, RowCount = 1, Margin = Padding.Empty };
        footer.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        footer.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100)); footer.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 245));
        footer.Controls.Add(Theme.Text("EARLY DEVELOPMENT  \u00b7  Local accounts  \u00b7  Server started separately", 8, Theme.Muted, FontStyle.Regular), 0, 0);
        var support = new LinkLabel { Text = "\u2661  Support Pyrealium on Patreon  \u2197", Dock = DockStyle.Fill, TextAlign = ContentAlignment.MiddleRight, LinkColor = Theme.Gold, ActiveLinkColor = Theme.Accent, VisitedLinkColor = Theme.Gold, Font = new Font("Segoe UI", 9), LinkBehavior = LinkBehavior.HoverUnderline, AccessibleName = "Support Vanquish90 on Patreon" };
        support.LinkClicked += delegate { try { Process.Start(new ProcessStartInfo("https://www.patreon.com/Vanquish90/") { UseShellExecute = true }); } catch (Exception) { status.Text = "Open patreon.com/Vanquish90 in your browser to support the project."; } }; footer.Controls.Add(support, 1, 0); return footer;
    }
    private Label Caption(string value) { return Theme.Text(value, 8, Theme.Muted, FontStyle.Bold); }
    private void StyleInput(TextBox box) { box.BorderStyle = BorderStyle.None; box.BackColor = Theme.Field; box.ForeColor = Theme.Parchment; box.Font = new Font("Segoe UI", 11); box.Dock = DockStyle.Fill; }
    private Control Input(Control control) { var frame = new Panel { Dock = DockStyle.Fill, BackColor = Theme.Field, Padding = new Padding(12, 9, 9, 7), Margin = new Padding(0, 0, 0, 5) }; frame.Controls.Add(control); return frame; }
    private Control PathRow(TextBox box, bool directory) {
        StyleInput(box); box.AccessibleName = directory ? "Game assets folder" : "Build 7148 executable";
        var row = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1, Margin = Padding.Empty };
        row.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        row.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100)); row.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 66));
        var frame = Input(box); frame.Margin = new Padding(0, 0, 6, 5); row.Controls.Add(frame, 0, 0);
        var browse = Theme.Button("Browse", false); browse.Font = new Font("Segoe UI", 8); browse.Margin = new Padding(0, 0, 0, 5); row.Controls.Add(browse, 1, 0);
        browse.Click += delegate {
            if (directory) { using (var dialog = new FolderBrowserDialog { Description = "Select the AC2 folder containing portal.dat and ijl15.dll", SelectedPath = box.Text, ShowNewFolderButton = false }) if (dialog.ShowDialog(this) == DialogResult.OK) box.Text = dialog.SelectedPath; }
            else { using (var dialog = new OpenFileDialog { Title = "Select the recovered build 7148 client", Filter = "AC2 client executable (*.exe)|*.exe", CheckFileExists = true }) if (dialog.ShowDialog(this) == DialogResult.OK) box.Text = dialog.FileName; }
        }; return row;
    }
    internal void ShowSettings(bool visible) {
        settingsVisible = visible; settingsPage.Visible = visible; playPage.Visible = !visible;
        playTab.ForeColor = visible ? Theme.Muted : Theme.Accent; settingsTab.ForeColor = visible ? Theme.Accent : Theme.Muted;
        AcceptButton = visible ? null : connect;
    }
    protected override bool ProcessCmdKey(ref Message msg, Keys keyData) {
        if (keyData == Keys.Escape && settingsVisible) { ShowSettings(false); playTab.Focus(); return true; }
        return base.ProcessCmdKey(ref msg, keyData);
    }
    private async void Connect(object sender, EventArgs args) {
        connect.Enabled = false; register.Enabled = false; status.ForeColor = Theme.Muted;
        try {
            var s = new Settings { Account = account.Text.Trim(), Host = host.Text.Trim(), Port = (int)port.Value, GameDirectory = game.Text.Trim(), ClientExecutable = client.Text.Trim(), AccountsUrl = accountsUrl.Text.Trim() };
            string error = Launcher.ValidateConnection(s);
            if (error == null) { error = Launcher.ValidateFiles(s); if (error != null) ShowSettings(true); }
            if (error != null) { status.ForeColor = Theme.Gold; status.Text = error; return; }
            if (Process.GetProcessesByName(Path.GetFileNameWithoutExtension(s.ClientExecutable)).Length > 0) { status.Text = "Close your existing AC2 client before launching another."; return; }
            string saveWarning = ""; try { Launcher.Save(s); } catch (Exception) { saveWarning = " Settings could not be saved."; }
            string secret = password.Text;
            status.Text = secret.Length == 0 ? "Starting client..." : "Signing in and starting client...";
            await Task.Run(delegate { AccountLogin.Launch(s, secret); });
            status.ForeColor = Theme.Accent; status.Text = "Client started. Check the game window for connection progress." + saveWarning;
        } catch (System.ComponentModel.Win32Exception ex) { status.Text = ex.NativeErrorCode == 1223 ? "Launch cancelled. You can try again when ready." : "Could not start the client: " + ex.Message; }
        catch (Exception ex) { status.Text = "Could not start the client: " + ex.Message; }
        finally { password.Clear(); connect.Enabled = true; register.Enabled = true; }
    }
    private async void Register(object sender, EventArgs args) {
        var s = new Settings { Account = account.Text.Trim().ToLowerInvariant(), Host = host.Text.Trim(), Port = (int)port.Value, AccountsUrl = accountsUrl.Text.Trim() };
        string error = Launcher.ValidateConnection(s);
        if (error != null) { status.Text = error; return; }
        if (password.Text.Length < 12) { status.Text = "Enter a new account name and a password of 12-128 characters first."; return; }
        using (var dialog = new Form { Text = "Confirm new account password", ClientSize = new Size(370, 135), FormBorderStyle = FormBorderStyle.FixedDialog, StartPosition = FormStartPosition.CenterParent, MinimizeBox = false, MaximizeBox = false, BackColor = Theme.Panel, ForeColor = Theme.Parchment }) {
            var label = new Label { Text = "Re-enter the password for " + s.Account, Left = 15, Top = 15, Width = 340 };
            var confirm = new TextBox { Left = 15, Top = 43, Width = 340, UseSystemPasswordChar = true, MaxLength = 128 };
            var ok = new Button { Text = "Create account", Left = 225, Top = 85, Width = 130, DialogResult = DialogResult.OK };
            dialog.Controls.Add(label); dialog.Controls.Add(confirm); dialog.Controls.Add(ok); dialog.AcceptButton = ok;
            if (dialog.ShowDialog(this) != DialogResult.OK) return;
            if (confirm.Text != password.Text) { status.Text = "Passwords did not match. Try again."; return; }
            confirm.Clear();
        }
        connect.Enabled = false; register.Enabled = false;
        try {
            string secret = password.Text; status.Text = "Creating your account...";
            await Task.Run(delegate { AccountLogin.Register(s, secret); });
            account.Text = s.Account; status.ForeColor = Theme.Accent;
            status.Text = "Account created. Enter your password and click Enter Dereth.";
        } catch (Exception ex) { status.ForeColor = Theme.Gold; status.Text = ex.Message; }
        finally { password.Clear(); connect.Enabled = true; register.Enabled = true; }
    }

}
