using System;
using System.Diagnostics;
using System.IO;
using System.Windows.Forms;
using Microsoft.Win32;

static class Uninstall {
    const string UninstallKey = @"Software\Microsoft\Windows\CurrentVersion\Uninstall\QuietNext";

    [STAThread]
    static void Main(string[] args) {
        bool silent = false;
        foreach (string a in args) if (string.Equals(a, "/S", StringComparison.OrdinalIgnoreCase)) silent = true;
        string dir = null;
        try {
            using (RegistryKey k = Registry.CurrentUser.OpenSubKey(UninstallKey)) {
                if (k != null) dir = k.GetValue("InstallLocation") as string;
            }
        } catch { }
        if (string.IsNullOrEmpty(dir)) dir = AppDomain.CurrentDomain.BaseDirectory.TrimEnd('\\');

        if (!silent && MessageBox.Show("Uninstall QuietNext? Your notes and library will be kept.", "QuietNext", MessageBoxButtons.YesNo, MessageBoxIcon.Question) != DialogResult.Yes) return;

        foreach (Process p in Process.GetProcessesByName("QuietNext")) {
            try { p.CloseMainWindow(); if (!p.WaitForExit(3000)) p.Kill(); } catch { }
        }

        try {
            string programs = Environment.GetFolderPath(Environment.SpecialFolder.Programs);
            string desk = Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory);
            TryDelete(Path.Combine(programs, "QuietNext.lnk"));
            TryDelete(Path.Combine(desk, "QuietNext.lnk"));
        } catch { }
        try { Registry.CurrentUser.DeleteSubKeyTree(UninstallKey, false); } catch { }

        string[] files = { "QuietNext.exe", "vcruntime140.dll", "vcruntime140_1.dll", "LICENSE", "THIRD_PARTY.md" };
        foreach (string f in files) TryDelete(Path.Combine(dir, f));

        string self = Path.Combine(dir, "uninstall.exe");
        string cmd = "/c timeout /t 2 >nul & del /f /q \"" + self + "\" & rmdir \"" + dir + "\"";
        try {
            ProcessStartInfo info = new ProcessStartInfo("cmd.exe", cmd);
            info.CreateNoWindow = true;
            info.UseShellExecute = false;
            info.WindowStyle = ProcessWindowStyle.Hidden;
            Process.Start(info);
        } catch {
            try { File.Delete(self); } catch { }
        }

        if (!silent) MessageBox.Show("QuietNext has been removed. Your notes are still in your library.", "QuietNext", MessageBoxButtons.OK, MessageBoxIcon.Information);
    }

    static void TryDelete(string path) {
        try { if (File.Exists(path)) File.Delete(path); } catch { }
    }
}
