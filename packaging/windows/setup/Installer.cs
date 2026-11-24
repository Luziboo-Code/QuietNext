using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Reflection;
using System.Threading;
using System.Windows.Forms;
using Microsoft.Win32;

static class Setup {
    const string AppName = "QuietNext";
    const string AppExe = "QuietNext.exe";
    const string UninstallKey = @"Software\Microsoft\Windows\CurrentVersion\Uninstall\QuietNext";

    static Assembly Self { get { return Assembly.GetExecutingAssembly(); } }

    static bool Has(string[] args, string want) {
        foreach (string a in args) if (string.Equals(a, want, StringComparison.OrdinalIgnoreCase)) return true;
        return false;
    }

    static string ArgValue(string[] args, string prefix) {
        foreach (string a in args) if (a.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)) return a.Substring(prefix.Length).Trim('"');
        return null;
    }

    public static string DefaultDir {
        get { return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"Programs\QuietNext"); }
    }

    [STAThread]
    static void Main(string[] args) {
        bool silent = Has(args, "/S") || Has(args, "/silent") || Has(args, "/UPDATE=1");
        bool update = Has(args, "/UPDATE=1");
        string dir = ArgValue(args, "/D=") ?? DefaultDir;
        try {
            if (silent) {
                Install(dir, false, update || Has(args, "/LAUNCH"), null);
                Environment.Exit(0);
            }
        } catch (Exception ex) {
            if (!silent) MessageBox.Show(ex.Message, AppName + " Setup", MessageBoxButtons.OK, MessageBoxIcon.Error);
            Environment.Exit(1);
        }
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new SetupForm(dir, update));
    }

    public static void Install(string dir, bool desktop, bool launch, Action<int, string> progress) {
        dir = Path.GetFullPath(dir);
        Directory.CreateDirectory(dir);
        Step(progress, 5, "Preparing...");

        CloseRunningApp(dir);

        string app = Path.Combine(dir, AppExe);
        Extract("QuietNext.app.gz", app, progress, 15);
        Extract("QuietNext.runtime.gz", Path.Combine(dir, "vcruntime140.dll"), progress, 55);
        Extract("QuietNext.runtime1.gz", Path.Combine(dir, "vcruntime140_1.dll"), progress, 70);
        Extract("QuietNext.uninstall.gz", Path.Combine(dir, "uninstall.exe"), progress, 80);

        Step(progress, 86, "Creating shortcuts...");
        string programs = Environment.GetFolderPath(Environment.SpecialFolder.Programs);
        Shortcut(Path.Combine(programs, AppName + ".lnk"), app);
        if (desktop) {
            string desk = Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory);
            Shortcut(Path.Combine(desk, AppName + ".lnk"), app);
        }
        Register(dir, app);
        Step(progress, 96, "Done");

        if (launch) {
            try { Process.Start(new ProcessStartInfo(app) { WorkingDirectory = dir, UseShellExecute = true }); } catch { }
        }
    }

    static void Step(Action<int, string> progress, int pct, string text) {
        if (progress != null) progress(pct, text);
    }

    static void CloseRunningApp(string dir) {
        string exe = Path.GetFullPath(Path.Combine(dir, AppExe));
        foreach (Process p in Process.GetProcessesByName("QuietNext")) {
            try {
                if (string.Equals(Path.GetFullPath(p.MainModule.FileName), exe, StringComparison.OrdinalIgnoreCase)) {
                    p.CloseMainWindow();
                    if (!p.WaitForExit(4000)) p.Kill();
                    p.WaitForExit(2000);
                }
            } catch { }
        }
    }

    static void Extract(string resource, string dest, Action<int, string> progress, int pct) {
        Step(progress, pct, "Installing " + Path.GetFileName(dest) + "...");
        Stream raw = Self.GetManifestResourceStream(resource);
        if (raw == null) throw new InvalidOperationException("Installer payload is missing: " + resource);
        string temp = dest + ".new";
        using (Stream gz = new GZipStream(raw, CompressionMode.Decompress))
        using (FileStream f = File.Create(temp)) gz.CopyTo(f);
        for (int i = 0; ; i++) {
            try {
                File.Copy(temp, dest, true);
                File.Delete(temp);
                return;
            } catch (IOException) {
                if (i >= 40) throw new IOException("Could not write " + dest + ". Close QuietNext and try again.");
                Thread.Sleep(250);
            } catch (UnauthorizedAccessException) {
                if (i >= 40) throw new IOException("Could not write " + dest + ". Close QuietNext and try again.");
                Thread.Sleep(250);
            }
        }
    }

    static void Shortcut(string path, string target) {
        Type shellType = Type.GetTypeFromProgID("WScript.Shell");
        object shell = Activator.CreateInstance(shellType);
        object link = shellType.InvokeMember("CreateShortcut", BindingFlags.InvokeMethod, null, shell, new object[] { path });
        Type t = link.GetType();
        t.InvokeMember("TargetPath", BindingFlags.SetProperty, null, link, new object[] { target });
        t.InvokeMember("WorkingDirectory", BindingFlags.SetProperty, null, link, new object[] { Path.GetDirectoryName(target) });
        t.InvokeMember("IconLocation", BindingFlags.SetProperty, null, link, new object[] { target + ",0" });
        t.InvokeMember("Description", BindingFlags.SetProperty, null, link, new object[] { "QuietNext" });
        t.InvokeMember("Save", BindingFlags.InvokeMethod, null, link, null);
    }

    static void Register(string dir, string app) {
        using (RegistryKey k = Registry.CurrentUser.CreateSubKey(UninstallKey)) {
            k.SetValue("DisplayName", "QuietNext", RegistryValueKind.String);
            k.SetValue("DisplayVersion", BuildInfo.Version, RegistryValueKind.String);
            k.SetValue("Publisher", "QuietNext", RegistryValueKind.String);
            k.SetValue("InstallLocation", dir, RegistryValueKind.String);
            k.SetValue("DisplayIcon", app, RegistryValueKind.String);
            k.SetValue("UninstallString", "\"" + Path.Combine(dir, "uninstall.exe") + "\"", RegistryValueKind.String);
            k.SetValue("NoModify", 1, RegistryValueKind.DWord);
            k.SetValue("NoRepair", 1, RegistryValueKind.DWord);
            try { k.SetValue("EstimatedSize", (int)(new FileInfo(app).Length / 1024), RegistryValueKind.DWord); } catch { }
        }
    }
}

class SetupForm : Form {
    readonly string dir;
    readonly bool update;
    readonly TextBox path;
    readonly CheckBox desktop;
    readonly CheckBox run;
    readonly Button install;
    readonly Button cancel;
    readonly ProgressBar bar;
    readonly Label status;

    public SetupForm(string dir, bool update) {
        this.dir = dir;
        this.update = update;
        Text = "QuietNext Setup";
        ClientSize = new Size(520, 300);
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox = false;
        MinimizeBox = false;
        StartPosition = FormStartPosition.CenterScreen;
        try { Icon = Icon.ExtractAssociatedIcon(Application.ExecutablePath); } catch { }

        Label title = new Label();
        title.Text = "QuietNext " + BuildInfo.Version;
        title.Font = new Font("Segoe UI", 16F, FontStyle.Regular);
        title.SetBounds(24, 20, 460, 36);
        Controls.Add(title);

        Label note = new Label();
        note.Text = "A quiet place for your notes. Your library is stored separately.";
        note.ForeColor = Color.FromArgb(90, 94, 100);
        note.SetBounds(26, 60, 460, 22);
        Controls.Add(note);

        Label place = new Label();
        place.Text = "Install folder";
        place.SetBounds(26, 104, 100, 22);
        Controls.Add(place);

        path = new TextBox();
        path.Text = dir;
        path.SetBounds(26, 126, 390, 26);
        Controls.Add(path);

        Button browse = new Button();
        browse.Text = "Browse...";
        browse.SetBounds(424, 125, 70, 28);
        browse.Click += delegate {
            FolderBrowserDialog d = new FolderBrowserDialog();
            d.SelectedPath = path.Text;
            if (d.ShowDialog(this) == DialogResult.OK) path.Text = d.SelectedPath;
        };
        Controls.Add(browse);

        desktop = new CheckBox();
        desktop.Text = "Create a desktop shortcut";
        desktop.SetBounds(26, 166, 300, 24);
        Controls.Add(desktop);

        run = new CheckBox();
        run.Text = "Launch QuietNext when finished";
        run.Checked = true;
        run.SetBounds(26, 192, 300, 24);
        Controls.Add(run);

        bar = new ProgressBar();
        bar.SetBounds(26, 226, 468, 16);
        Controls.Add(bar);

        status = new Label();
        status.Text = "Ready to install.";
        status.ForeColor = Color.FromArgb(90, 94, 100);
        status.SetBounds(26, 246, 300, 22);
        Controls.Add(status);

        install = new Button();
        install.Text = update ? "Update" : "Install";
        install.SetBounds(322, 264, 82, 30);
        install.Click += delegate { Start(); };
        Controls.Add(install);

        cancel = new Button();
        cancel.Text = "Cancel";
        cancel.SetBounds(412, 264, 82, 30);
        cancel.Click += delegate { Close(); };
        Controls.Add(cancel);
        AcceptButton = install;
        CancelButton = cancel;
    }

    void Start() {
        path.Enabled = desktop.Enabled = run.Enabled = install.Enabled = cancel.Enabled = false;
        string dest = path.Text.Trim();
        if (dest.Length == 0) dest = Setup.DefaultDir;
        bar.Value = 3;
        Thread t = new Thread(delegate() {
            try {
                Setup.Install(dest, desktop.Checked, run.Checked, delegate(int pct, string text) {
                    try {
                        BeginInvoke((Action)delegate {
                            bar.Value = Math.Max(0, Math.Min(100, pct));
                            status.Text = text;
                        });
                    } catch { }
                });
                try {
                    BeginInvoke((Action)delegate {
                        bar.Value = 100;
                        status.Text = "Installed.";
                        install.Text = "Finish";
                        install.Enabled = true;
                        install.Click += delegate { Close(); };
                    });
                } catch { }
            } catch (Exception ex) {
                try {
                    BeginInvoke((Action)delegate {
                        MessageBox.Show(this, ex.Message, "QuietNext Setup", MessageBoxButtons.OK, MessageBoxIcon.Error);
                        path.Enabled = desktop.Enabled = run.Enabled = install.Enabled = cancel.Enabled = true;
                        status.Text = "Installation failed.";
                    });
                } catch { }
            }
        });
        t.IsBackground = true;
        t.Start();
    }
}
