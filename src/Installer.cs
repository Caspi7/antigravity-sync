using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Net;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using System.Windows.Forms;
using Microsoft.Win32;

[assembly: AssemblyTitle("Antigravity Sync Setup")]
[assembly: AssemblyDescription("Installer for Antigravity Sync v2.0.5")]
[assembly: AssemblyVersion("2.0.5.0")]
[assembly: AssemblyFileVersion("2.0.5.0")]
[assembly: AssemblyInformationalVersion("2.0.5")]

namespace AntigravitySyncSetup
{
    public class InstallOptions
    {
        public string InstallDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            @"Programs\AntigravitySync");
        public bool CreateDesktopShortcuts = true;
        public bool CreateStartMenuShortcut = true;
        public bool AutoCheckUpdates = true;
        public bool LaunchAfterInstall = true;
    }

    public class ReleaseCheckResult
    {
        public string LatestVersion;
        public string InstallerUrl;
        public string Sha256Url;
    }

    static class InstallerCore
    {
        public const string AppVersion = "2.0.5";
        public const string RepoOwner = "Caspi7";
        public const string RepoName = "antigravity-sync";
        public const string LatestReleaseApiUrl = "https://api.github.com/repos/" + RepoOwner + "/" + RepoName + "/releases/latest";
        public const string ReleasesPageUrl = "https://github.com/" + RepoOwner + "/" + RepoName + "/releases";

        public static void Install(InstallOptions opts, Action<int, string> report)
        {
            report(10, "Closing running instances...");
            foreach (var p in Process.GetProcessesByName("AntigravitySync"))
            {
                try { p.Kill(); p.WaitForExit(5000); } catch { }
            }

            report(30, "Extracting AntigravitySync.exe...");
            Directory.CreateDirectory(opts.InstallDir);
            string exePath = Path.Combine(opts.InstallDir, "AntigravitySync.exe");

            using (Stream res = Assembly.GetExecutingAssembly().GetManifestResourceStream("AntigravitySync.exe.gz"))
            {
                if (res == null) throw new Exception("Embedded payload AntigravitySync.exe.gz not found.");
                using (var gz = new GZipStream(res, CompressionMode.Decompress))
                using (var fs = File.Create(exePath))
                {
                    gz.CopyTo(fs);
                }
            }

            if (opts.CreateDesktopShortcuts)
            {
                report(55, "Creating Desktop shortcuts...");
                string desktop = Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory);
                CreateShortcut(Path.Combine(desktop, "Antigravity Sync.lnk"), exePath, "", "Open Antigravity Sync");
                CreateShortcut(Path.Combine(desktop, "Push Antigravity.lnk"), exePath, "--push --relaunch", "Push Antigravity data to cloud drive");
                CreateShortcut(Path.Combine(desktop, "Pull Antigravity.lnk"), exePath, "--pull --relaunch", "Pull Antigravity data from cloud drive");
            }

            if (opts.CreateStartMenuShortcut)
            {
                report(70, "Creating Start Menu shortcut...");
                string startMenu = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Programs), "Antigravity Sync");
                Directory.CreateDirectory(startMenu);
                CreateShortcut(Path.Combine(startMenu, "Antigravity Sync.lnk"), exePath, "", "Open Antigravity Sync");
            }

            report(80, "Saving update preferences...");
            PersistAutoUpdatePreference(opts.AutoCheckUpdates);

            report(90, "Registering in Windows Apps & Features...");
            using (var key = Registry.CurrentUser.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Uninstall\AntigravitySync"))
            {
                key.SetValue("DisplayName", "Antigravity Sync");
                key.SetValue("DisplayVersion", AppVersion);
                key.SetValue("Publisher", RepoOwner);
                key.SetValue("InstallLocation", opts.InstallDir);
                key.SetValue("DisplayIcon", exePath);
                key.SetValue("UninstallString", "cmd.exe /c rmdir /s /q \"" + opts.InstallDir + "\"");
                key.SetValue("NoModify", 1, RegistryValueKind.DWord);
                key.SetValue("NoRepair", 1, RegistryValueKind.DWord);
            }

            report(100, "Installation complete!");
        }

        private static void PersistAutoUpdatePreference(bool autoCheckUpdates)
        {
            try
            {
                string dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "AntigravitySync");
                Directory.CreateDirectory(dir);
                string settingsPath = Path.Combine(dir, "settings.json");
                string boolVal = autoCheckUpdates ? "true" : "false";

                if (File.Exists(settingsPath))
                {
                    string json = File.ReadAllText(settingsPath, Encoding.UTF8);
                    if (Regex.IsMatch(json, "\"auto_check_updates\"\\s*:\\s*(true|false)", RegexOptions.IgnoreCase))
                    {
                        json = Regex.Replace(json, "\"auto_check_updates\"\\s*:\\s*(true|false)", "\"auto_check_updates\": " + boolVal, RegexOptions.IgnoreCase);
                        File.WriteAllText(settingsPath, json, Encoding.UTF8);
                    }
                }
                else
                {
                    string json = "{\n  \"auto_check_updates\": " + boolVal + "\n}\n";
                    File.WriteAllText(settingsPath, json, Encoding.UTF8);
                }
            }
            catch { }
        }

        public static ReleaseCheckResult CheckForNewerInstaller()
        {
            ServicePointManager.SecurityProtocol = SecurityProtocolType.Tls12;
            using (var wc = new WebClient())
            {
                wc.Headers.Add("User-Agent", "AntigravitySyncSetup/" + AppVersion);
                wc.Headers.Add("Accept", "application/vnd.github+json");
                string json = wc.DownloadString(LatestReleaseApiUrl);

                var tagMatch = Regex.Match(json, "\"tag_name\"\\s*:\\s*\"v?([^\"]+)\"");
                if (!tagMatch.Success) return null;
                string latest = tagMatch.Groups[1].Value.Trim();

                if (!IsNewerVersion(latest, AppVersion)) return null;

                var exeMatch = Regex.Match(json, "\"browser_download_url\"\\s*:\\s*\"([^\"]+Setup[^\"]*\\.exe)\"", RegexOptions.IgnoreCase);
                var shaMatch = Regex.Match(json, "\"browser_download_url\"\\s*:\\s*\"([^\"]+SHA256SUMS\\.txt)\"", RegexOptions.IgnoreCase);
                if (!exeMatch.Success) return null;

                return new ReleaseCheckResult
                {
                    LatestVersion = latest,
                    InstallerUrl = exeMatch.Groups[1].Value,
                    Sha256Url = shaMatch.Success ? shaMatch.Groups[1].Value : null
                };
            }
        }

        public static string DownloadAndVerifyInstaller(ReleaseCheckResult release, Action<int, string> report)
        {
            ServicePointManager.SecurityProtocol = SecurityProtocolType.Tls12;
            string tempExe = Path.Combine(Path.GetTempPath(), "AntigravitySync-Setup-v" + release.LatestVersion + ".exe");

            using (var wc = new WebClient())
            {
                wc.Headers.Add("User-Agent", "AntigravitySyncSetup/" + AppVersion);
                wc.DownloadProgressChanged += (s, e) =>
                {
                    int pct = Math.Min(90, e.ProgressPercentage);
                    report(pct, pct + "% — Downloading v" + release.LatestVersion + "...");
                };
                wc.DownloadFileTaskAsync(new Uri(release.InstallerUrl), tempExe).GetAwaiter().GetResult();
            }

            if (!string.IsNullOrEmpty(release.Sha256Url))
            {
                report(95, "95% — Verifying SHA-256 checksum...");
                using (var wc = new WebClient())
                {
                    wc.Headers.Add("User-Agent", "AntigravitySyncSetup/" + AppVersion);
                    string sums = wc.DownloadString(release.Sha256Url);
                    string actual = ComputeSha256(tempExe);
                    if (sums.IndexOf(actual, StringComparison.OrdinalIgnoreCase) < 0)
                    {
                        try { File.Delete(tempExe); } catch { }
                        throw new Exception("SHA-256 verification failed!");
                    }
                }
            }

            report(100, "Launching v" + release.LatestVersion + " installer...");
            return tempExe;
        }

        private static string ComputeSha256(string path)
        {
            using (var sha = SHA256.Create())
            using (var fs = File.OpenRead(path))
            {
                byte[] hash = sha.ComputeHash(fs);
                return BitConverter.ToString(hash).Replace("-", "").ToLowerInvariant();
            }
        }

        private static bool IsNewerVersion(string latest, string current)
        {
            Version vLatest, vCurrent;
            if (Version.TryParse(latest, out vLatest) && Version.TryParse(current, out vCurrent))
            {
                return vLatest > vCurrent;
            }
            return false;
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
        private readonly Color _bg = Color.FromArgb(15, 23, 42);
        private readonly Color _surface = Color.FromArgb(24, 35, 58);
        private readonly Color _border = Color.FromArgb(41, 58, 90);
        private readonly Color _fg = Color.FromArgb(241, 245, 249);
        private readonly Color _muted = Color.FromArgb(148, 163, 184);
        private readonly Color _accent = Color.FromArgb(0, 122, 204);
        private readonly Color _green = Color.FromArgb(34, 197, 94);

        private Label _lblVersionPill;
        private Panel _updateBanner;
        private Label _lblUpdateBanner;
        private Button _btnGetLatest;
        private TextBox _txtInstallDir;
        private Button _btnBrowse;
        private CheckBox _chkDesktop;
        private CheckBox _chkStartMenu;
        private CheckBox _chkAutoUpdate;
        private CheckBox _chkLaunch;
        private ProgressBar _bar;
        private Label _lblStatus;
        private Button _btnAction;

        private ReleaseCheckResult _newerRelease;
        private bool _isInstalled;

        public SetupForm()
        {
            Text = "Antigravity Sync Setup v" + InstallerCore.AppVersion;
            ClientSize = new Size(520, 395);
            FormBorderStyle = FormBorderStyle.FixedDialog;
            MaximizeBox = false;
            StartPosition = FormStartPosition.CenterScreen;
            Font = new Font("Segoe UI", 9.5f);
            BackColor = _bg;
            ForeColor = _fg;

            BuildUi();
            Shown += (s, e) => CheckForUpdatesInBackground();
        }

        private void BuildUi()
        {
            var title = new Label
            {
                Text = "Antigravity Sync Setup",
                Font = new Font("Segoe UI Semibold", 14.5f),
                Location = new Point(20, 16),
                AutoSize = true,
                ForeColor = _fg
            };

            _lblVersionPill = new Label
            {
                Text = "v" + InstallerCore.AppVersion + "  •  Checking for newer version...",
                Font = new Font("Segoe UI", 8.5f),
                Location = new Point(22, 46),
                AutoSize = true,
                ForeColor = _muted
            };

            // Update available banner (hidden unless newer release found)
            _updateBanner = new Panel
            {
                Location = new Point(20, 72),
                Size = new Size(480, 40),
                BackColor = Color.FromArgb(20, 56, 38),
                Visible = false
            };
            _lblUpdateBanner = new Label
            {
                Text = "A newer version is available!",
                Location = new Point(10, 11),
                Size = new Size(305, 20),
                ForeColor = Color.FromArgb(134, 239, 172),
                Font = new Font("Segoe UI Semibold", 9f)
            };
            _btnGetLatest = new Button
            {
                Text = "Download Latest",
                Location = new Point(322, 6),
                Size = new Size(148, 28),
                FlatStyle = FlatStyle.Flat,
                BackColor = _green,
                ForeColor = Color.White,
                Font = new Font("Segoe UI Semibold", 8.5f),
                Cursor = Cursors.Hand
            };
            _btnGetLatest.FlatAppearance.BorderSize = 0;
            _btnGetLatest.Click += OnDownloadLatestClick;
            _updateBanner.Controls.AddRange(new Control[] { _lblUpdateBanner, _btnGetLatest });

            // Options card
            var card = new Panel
            {
                Location = new Point(20, 120),
                Size = new Size(480, 182),
                BackColor = _surface
            };

            var lblDir = new Label
            {
                Text = "Install Location:",
                Location = new Point(14, 12),
                AutoSize = true,
                ForeColor = _muted,
                Font = new Font("Segoe UI Semibold", 8.5f)
            };

            var defaultOpts = new InstallOptions();
            _txtInstallDir = new TextBox
            {
                Text = defaultOpts.InstallDir,
                Location = new Point(14, 32),
                Size = new Size(368, 25),
                BackColor = _bg,
                ForeColor = _fg,
                BorderStyle = BorderStyle.FixedSingle
            };

            _btnBrowse = new Button
            {
                Text = "Browse...",
                Location = new Point(388, 31),
                Size = new Size(78, 26),
                FlatStyle = FlatStyle.Flat,
                BackColor = _bg,
                ForeColor = _fg,
                Cursor = Cursors.Hand
            };
            _btnBrowse.FlatAppearance.BorderColor = _border;
            _btnBrowse.Click += (s, e) =>
            {
                using (var dlg = new FolderBrowserDialog())
                {
                    dlg.SelectedPath = _txtInstallDir.Text;
                    if (dlg.ShowDialog(this) == DialogResult.OK)
                    {
                        _txtInstallDir.Text = dlg.SelectedPath;
                    }
                }
            };

            _chkDesktop = new CheckBox
            {
                Text = "Create Desktop shortcuts (Antigravity Sync, Push & Pull)",
                Checked = true,
                Location = new Point(14, 68),
                AutoSize = true,
                ForeColor = _fg,
                Cursor = Cursors.Hand
            };

            _chkStartMenu = new CheckBox
            {
                Text = "Create Start Menu shortcut",
                Checked = true,
                Location = new Point(14, 94),
                AutoSize = true,
                ForeColor = _fg,
                Cursor = Cursors.Hand
            };

            _chkAutoUpdate = new CheckBox
            {
                Text = "Automatically check for updates on startup",
                Checked = true,
                Location = new Point(14, 120),
                AutoSize = true,
                ForeColor = _fg,
                Cursor = Cursors.Hand
            };

            _chkLaunch = new CheckBox
            {
                Text = "Launch Antigravity Sync after installation",
                Checked = true,
                Location = new Point(14, 146),
                AutoSize = true,
                ForeColor = _fg,
                Cursor = Cursors.Hand
            };

            card.Controls.AddRange(new Control[]
            {
                lblDir, _txtInstallDir, _btnBrowse,
                _chkDesktop, _chkStartMenu, _chkAutoUpdate, _chkLaunch
            });

            _bar = new ProgressBar
            {
                Location = new Point(20, 314),
                Size = new Size(480, 16),
                Minimum = 0,
                Maximum = 100
            };

            _lblStatus = new Label
            {
                Text = "Ready to install Antigravity Sync v" + InstallerCore.AppVersion + ".",
                Location = new Point(20, 338),
                Size = new Size(340, 24),
                ForeColor = _muted
            };

            _btnAction = new Button
            {
                Text = "Install",
                Location = new Point(380, 342),
                Size = new Size(120, 36),
                FlatStyle = FlatStyle.Flat,
                BackColor = _accent,
                ForeColor = Color.White,
                Font = new Font("Segoe UI Semibold", 9.5f),
                Cursor = Cursors.Hand
            };
            _btnAction.FlatAppearance.BorderSize = 0;
            _btnAction.Click += OnInstallClick;

            Controls.AddRange(new Control[]
            {
                title, _lblVersionPill, _updateBanner, card, _bar, _lblStatus, _btnAction
            });
        }

        private void CheckForUpdatesInBackground()
        {
            Task.Run(() =>
            {
                try
                {
                    var latest = InstallerCore.CheckForNewerInstaller();
                    BeginInvoke((Action)(() =>
                    {
                        if (latest != null)
                        {
                            _newerRelease = latest;
                            _lblVersionPill.Text = "v" + InstallerCore.AppVersion + "  •  Newer version v" + latest.LatestVersion + " available";
                            _lblVersionPill.ForeColor = Color.FromArgb(245, 158, 11);
                            _lblUpdateBanner.Text = "Newer version v" + latest.LatestVersion + " is available on GitHub!";
                            _btnGetLatest.Text = "Download v" + latest.LatestVersion;
                            _updateBanner.Visible = true;
                        }
                        else
                        {
                            _lblVersionPill.Text = "v" + InstallerCore.AppVersion + "  •  Latest version";
                            _lblVersionPill.ForeColor = _green;
                        }
                    }));
                }
                catch
                {
                    BeginInvoke((Action)(() =>
                    {
                        _lblVersionPill.Text = "v" + InstallerCore.AppVersion + "  •  Offline (skipping version check)";
                        _lblVersionPill.ForeColor = _muted;
                    }));
                }
            });
        }

        private void OnDownloadLatestClick(object sender, EventArgs e)
        {
            if (_newerRelease == null) return;
            _btnGetLatest.Enabled = false;
            _btnAction.Enabled = false;

            Task.Run(() =>
            {
                try
                {
                    string exePath = InstallerCore.DownloadAndVerifyInstaller(_newerRelease, (pct, msg) =>
                    {
                        BeginInvoke((Action)(() =>
                        {
                            _bar.Value = Math.Max(0, Math.Min(100, pct));
                            _lblStatus.Text = msg;
                        }));
                    });
                    Process.Start(new ProcessStartInfo { FileName = exePath, UseShellExecute = true });
                    BeginInvoke((Action)(() => Close()));
                }
                catch (Exception ex)
                {
                    BeginInvoke((Action)(() =>
                    {
                        MessageBox.Show(this, "Could not download latest installer: " + ex.Message, "Update Error", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                        _btnGetLatest.Enabled = true;
                        _btnAction.Enabled = true;
                    }));
                }
            });
        }

        private void OnInstallClick(object sender, EventArgs e)
        {
            if (_isInstalled)
            {
                if (_chkLaunch.Checked)
                {
                    string exe = Path.Combine(_txtInstallDir.Text.Trim(), "AntigravitySync.exe");
                    if (File.Exists(exe)) Process.Start(new ProcessStartInfo { FileName = exe, UseShellExecute = true });
                }
                Close();
                return;
            }

            var opts = new InstallOptions
            {
                InstallDir = _txtInstallDir.Text.Trim(),
                CreateDesktopShortcuts = _chkDesktop.Checked,
                CreateStartMenuShortcut = _chkStartMenu.Checked,
                AutoCheckUpdates = _chkAutoUpdate.Checked,
                LaunchAfterInstall = _chkLaunch.Checked
            };

            _btnAction.Enabled = false;
            _btnBrowse.Enabled = false;
            _txtInstallDir.Enabled = false;
            _chkDesktop.Enabled = false;
            _chkStartMenu.Enabled = false;
            _chkAutoUpdate.Enabled = false;

            Task.Run(() =>
            {
                try
                {
                    InstallerCore.Install(opts, (pct, msg) =>
                    {
                        BeginInvoke((Action)(() =>
                        {
                            _bar.Value = Math.Max(0, Math.Min(100, pct));
                            _lblStatus.Text = msg;
                        }));
                    });
                    BeginInvoke((Action)(() =>
                    {
                        _isInstalled = true;
                        _btnAction.Text = "Finish";
                        _btnAction.BackColor = _green;
                        _btnAction.Enabled = true;
                    }));
                }
                catch (Exception ex)
                {
                    BeginInvoke((Action)(() =>
                    {
                        MessageBox.Show(this, "Installation failed: " + ex.Message, "Setup Error", MessageBoxButtons.OK, MessageBoxIcon.Error);
                        _btnAction.Enabled = true;
                        _btnBrowse.Enabled = true;
                        _txtInstallDir.Enabled = true;
                        _chkDesktop.Enabled = true;
                        _chkStartMenu.Enabled = true;
                        _chkAutoUpdate.Enabled = true;
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
                var opts = new InstallOptions();
                InstallerCore.Install(opts, (pct, msg) => { });
                string exe = Path.Combine(opts.InstallDir, "AntigravitySync.exe");
                if (File.Exists(exe)) Process.Start(new ProcessStartInfo { FileName = exe, UseShellExecute = true });
                return;
            }

            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            Application.Run(new SetupForm());
        }
    }
}
