// SPDX-License-Identifier: AGPL-3.0-only
using System;
using System.Drawing;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using System.Windows.Forms;

internal sealed class AccountRegistration : Form {
    private readonly TextBox username = new TextBox(), password = new TextBox(), confirmation = new TextBox();
    private readonly Label message = Theme.Text("Use 1-32 letters, numbers or underscores for your username.\nChoose a password of 12-128 characters.", 9, Theme.Muted, FontStyle.Regular);
    private readonly Button create = Theme.Button("CREATE ACCOUNT", true), cancel = Theme.Button("CANCEL", false);
    private readonly string accountsUrl;
    private bool busy;
    internal string AccountName { get; private set; }

    internal AccountRegistration(string url, string suggestedName) {
        accountsUrl = url;
        Text = "Create your Pyrealium account";
        ClientSize = new Size(440, 440); MinimumSize = new Size(440, 440);
        FormBorderStyle = FormBorderStyle.FixedDialog; StartPosition = FormStartPosition.CenterParent;
        MinimizeBox = false; MaximizeBox = false; BackColor = Theme.Panel; ForeColor = Theme.Parchment;
        AutoScaleDimensions = new SizeF(96, 96); AutoScaleMode = AutoScaleMode.Dpi;
        var layout = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, RowCount = 10, Padding = new Padding(22), Margin = Padding.Empty };
        foreach (int height in new[] { 37, 23, 42, 23, 42, 23, 42, 78, 42 }) layout.RowStyles.Add(new RowStyle(SizeType.Absolute, height));
        layout.RowStyles.Add(new RowStyle(SizeType.Percent, 100)); Controls.Add(layout);
        layout.Controls.Add(Theme.Text("Create your account", 19, Theme.Parchment, FontStyle.Bold), 0, 0);
        layout.Controls.Add(Theme.Text("USERNAME", 8, Theme.Muted, FontStyle.Bold), 0, 1);
        layout.Controls.Add(Field(username, "New account username", false), 0, 2);
        layout.Controls.Add(Theme.Text("PASSWORD", 8, Theme.Muted, FontStyle.Bold), 0, 3);
        layout.Controls.Add(Field(password, "New account password", true), 0, 4);
        layout.Controls.Add(Theme.Text("CONFIRM PASSWORD", 8, Theme.Muted, FontStyle.Bold), 0, 5);
        layout.Controls.Add(Field(confirmation, "Confirm new account password", true), 0, 6);
        message.AccessibleName = "Account creation status"; layout.Controls.Add(message, 0, 7);
        var actions = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1, Margin = Padding.Empty };
        actions.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 34)); actions.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 66));
        cancel.Margin = new Padding(0, 0, 8, 0); actions.Controls.Add(cancel, 0, 0); actions.Controls.Add(create, 1, 0); layout.Controls.Add(actions, 0, 8);
        username.MaxLength = 32;
        if (!String.Equals(suggestedName, "local", StringComparison.OrdinalIgnoreCase)) username.Text = suggestedName;
        create.Click += CreateAccount;
        cancel.Click += delegate { DialogResult = DialogResult.Cancel; Close(); };
        AcceptButton = create; CancelButton = cancel;
        Shown += delegate { username.Focus(); };
        FormClosing += delegate(object sender, FormClosingEventArgs args) { if (busy) args.Cancel = true; };
    }
    private Control Field(TextBox input, string accessibleName, bool secret) {
        input.AccessibleName = accessibleName; input.UseSystemPasswordChar = secret; input.MaxLength = secret ? 128 : 32;
        input.Font = new Font("Segoe UI", 11); input.BackColor = Theme.Field; input.ForeColor = Theme.Parchment; input.BorderStyle = BorderStyle.None; input.Dock = DockStyle.Fill;
        var frame = new Panel { Dock = DockStyle.Fill, BackColor = Theme.Field, Padding = new Padding(10, 8, 10, 7), Margin = new Padding(0, 0, 0, 4) };
        frame.Controls.Add(input); return frame;
    }
    private void Error(string text, Control focus) {
        message.ForeColor = Theme.Gold; message.Text = text; focus.Focus();
    }
    private async void CreateAccount(object sender, EventArgs args) {
        string name = username.Text.Trim().ToLowerInvariant();
        if (!Regex.IsMatch(name, @"\A[a-z0-9_]{1,32}\z")) { Error("Username needs 1-32 letters, numbers or underscores.", username); return; }
        if (name == "local" || name == "admin" || name == "administrator" || name == "system") { Error("That name is reserved. Choose a different username.", username); return; }
        if (password.Text.Length < 12 || password.Text.Length > 128) { Error("Choose a password of 12-128 characters.", password); return; }
        if (password.Text != confirmation.Text) { Error("The passwords do not match. Please re-enter the confirmation.", confirmation); return; }
        busy = true; create.Enabled = false; cancel.Enabled = false; username.Enabled = false; password.Enabled = false; confirmation.Enabled = false;
        message.ForeColor = Theme.Muted; message.Text = "Creating your account...";
        bool completed = false;
        try {
            string secret = password.Text;
            var settings = new Settings { Account = name, AccountsUrl = accountsUrl };
            await Task.Run(delegate { AccountLogin.Register(settings, secret); });
            AccountName = name; completed = true;
        } catch (Exception error) { message.ForeColor = Theme.Gold; message.Text = error.Message; }
        finally {
            busy = false; create.Enabled = true; cancel.Enabled = true; username.Enabled = true; password.Enabled = true; confirmation.Enabled = true;
        }
        if (completed) { password.Clear(); confirmation.Clear(); DialogResult = DialogResult.OK; Close(); }
    }
    protected override void Dispose(bool disposing) {
        if (disposing) { password.Clear(); confirmation.Clear(); }
        base.Dispose(disposing);
    }
}
