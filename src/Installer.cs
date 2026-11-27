using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Reflection;
using System.Threading.Tasks;
using System.Windows.Forms;
using Microsoft.Win32;

[assembly: AssemblyTitle("Antigravity Sync Setup")]
[assembly: AssemblyDescription("Installer for Antigravity Sync v1.0.1")]
[assembly: AssemblyVersion("1.0.1.0")]
[assembly: AssemblyFileVersion("1.0.1.0")]
[assembly: AssemblyInformationalVersion("1.0.1")]

namespace AntigravitySyncSetup
{
    static class InstallerCore
    {
        public const string AppVersion = "1.0.1";
        public static string InstallDir
        {
            get { return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"Programs\AntigravitySync"); }
        }

        public static void Install(Action<int, string> report)
        {
            report(10, "Closing running instances...");
            foreach (var p in Process.GetProcessesByName("AntigravitySync"))
            {
                try { p.Kill(); p.WaitForExit(5000); } catch { }
            }

            report(30, "Extracting AntigravitySync.exe...");
            Directory.CreateDirectory(InstallDir);
            string exePath = Path.Combine(InstallDir, "AntigravitySync.exe");

            using (Stream res = Assembly.GetExecutingAssembly().GetManifestResourceStream("AntigravitySync.exe.gz"))
            {
                if (res == null) throw new Exception("Embedded payload AntigravitySync.exe.gz not found.");
                using (var gz = new GZipStream(res, CompressionMode.Decompress))
                using (var fs = File.Create(exePath))
                {
                    gz.CopyTo(fs);
                }
            }

            report(65, "Creating Desktop & Start Menu shortcuts...");
            string desktop = Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory);
            string startMenu = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Programs), "Antigravity Sync");
            Directory.CreateDirectory(startMenu);

            CreateShortcut(Path.Combine(desktop, "Antigravity Sync.lnk"), exePath, "", "Open Antigravity Sync");
            CreateShortcut(Path.Combine(desktop, "Push Antigravity.lnk"), exePath, "--push --relaunch", "Push Antigravity data to cloud drive");
            CreateShortcut(Path.Combine(desktop, "Pull Antigravity.lnk"), exePath, "--pull --relaunch", "Pull Antigravity data from cloud drive");
            CreateShortcut(Path.Combine(startMenu, "Antigravity Sync.lnk"), exePath, "", "Open Antigravity Sync");

            report(85, "Registering in Windows Apps & Features...");
            using (var key = Registry.CurrentUser.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Uninstall\AntigravitySync"))
            {
                key.SetValue("DisplayName", "Antigravity Sync");
                key.SetValue("DisplayVersion", AppVersion);
                key.SetValue("Publisher", "Caspi7");
                key.SetValue("InstallLocation", InstallDir);
                key.SetValue("DisplayIcon", exePath);
                key.SetValue("UninstallString", "cmd.exe /c rmdir /s /q \"" + InstallDir + "\"");
                key.SetValue("NoModify", 1, RegistryValueKind.DWord);
                key.SetValue("NoRepair", 1, RegistryValueKind.DWord);
            }

            report(100, "Installation complete!");
        }

        private static void CreateShortcut(string shortcutPath, string targetPath, string arguments, string description)
        {
            Type shellType = Type.GetTypeFromProgID("WScript.Shell");
            dynamic shell = Activator.CreateInstance(shellType);
            dynamic shortcut = shell.CreateShortcut(shortcutPath);
            shortcut.TargetPath = targetPath;
            shortcut.Arguments = arguments;
            shortcut.WorkingDirectory = Path.GetDirectoryName(targetPath);
            shortcut.Description = description;
            shortcut.IconLocation = targetPath + ",0";
            shortcut.Save();
        }
    }

    public class SetupForm : Form
    {
        private ProgressBar _bar;
        private Label _lbl;
        private Button _btnAction;

        public SetupForm()
        {
            Text = "Antigravity Sync Setup v" + InstallerCore.AppVersion;
            ClientSize = new Size(460, 190);
            FormBorderStyle = FormBorderStyle.FixedDialog;
            MaximizeBox = false;
            StartPosition = FormStartPosition.CenterScreen;
            Font = new Font("Segoe UI", 9.5f);
            BackColor = Color.FromArgb(24, 24, 27);
            ForeColor = Color.FromArgb(244, 244, 245);

            var title = new Label
            {
                Text = "Install Antigravity Sync v" + InstallerCore.AppVersion,
                Font = new Font("Segoe UI Semibold", 13f),
                Location = new Point(20, 18),
                AutoSize = true
            };

            _lbl = new Label
            {
                Text = "Installs Antigravity Sync and creates Push/Pull Desktop shortcuts.",
                Location = new Point(22, 54),
                Size = new Size(416, 24),
                ForeColor = Color.FromArgb(161, 161, 170)
            };

            _bar = new ProgressBar
            {
                Location = new Point(22, 90),
                Size = new Size(416, 20),
                Minimum = 0,
                Maximum = 100
            };

            _btnAction = new Button
            {
                Text = "Install",
                Location = new Point(328, 132),
                Size = new Size(110, 34),
                FlatStyle = FlatStyle.Flat,
                BackColor = Color.FromArgb(34, 197, 94),
                ForeColor = Color.White,
                Font = new Font("Segoe UI Semibold", 9.5f)
            };
            _btnAction.FlatAppearance.BorderSize = 0;
            _btnAction.Click += OnInstallClick;

            Controls.AddRange(new Control[] { title, _lbl, _bar, _btnAction });
        }

        private void OnInstallClick(object sender, EventArgs e)
        {
            if (_btnAction.Text == "Finish")
            {
                string exe = Path.Combine(InstallerCore.InstallDir, "AntigravitySync.exe");
                if (File.Exists(exe)) Process.Start(new ProcessStartInfo { FileName = exe, UseShellExecute = true });
                Close();
                return;
            }

            _btnAction.Enabled = false;
            Task.Run(() =>
            {
                try
                {
                    InstallerCore.Install((pct, msg) =>
                    {
                        BeginInvoke((Action)(() =>
                        {
                            _bar.Value = pct;
                            _lbl.Text = msg;
                        }));
                    });
                    BeginInvoke((Action)(() =>
                    {
                        _btnAction.Text = "Finish";
                        _btnAction.Enabled = true;
                    }));
                }
                catch (Exception ex)
                {
                    BeginInvoke((Action)(() =>
                    {
                        MessageBox.Show(this, "Installation failed: " + ex.Message, "Setup Error", MessageBoxButtons.OK, MessageBoxIcon.Error);
                        _btnAction.Enabled = true;
                    }));
                }
            });
        }
    }

    static class Program
    {
        [STAThread]
        static void Main(string[] args)
        {
            bool silent = false;
            foreach (string a in args)
            {
                if (string.Equals(a, "/S", StringComparison.OrdinalIgnoreCase) || string.Equals(a, "--silent", StringComparison.OrdinalIgnoreCase))
                    silent = true;
            }

            if (silent)
            {
                InstallerCore.Install((pct, msg) => { });
                string exe = Path.Combine(InstallerCore.InstallDir, "AntigravitySync.exe");
                if (File.Exists(exe)) Process.Start(new ProcessStartInfo { FileName = exe, UseShellExecute = true });
                return;
            }

            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            Application.Run(new SetupForm());
        }
    }
}
