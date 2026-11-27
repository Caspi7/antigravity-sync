using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Net;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;
using Microsoft.Win32;

[assembly: AssemblyTitle("Antigravity Sync")]
[assembly: AssemblyDescription("One-click Google Drive / Cloud sync for Antigravity")]
[assembly: AssemblyVersion("1.0.1.0")]
[assembly: AssemblyFileVersion("1.0.1.0")]
[assembly: AssemblyInformationalVersion("1.0.1")]

namespace AntigravitySync
{
    public class AppSettings
    {
        public string SyncRoot = @"G:\My Drive\AntigravitySync";
        public string Theme = "System"; // System, Light, Dark, Dark Blue
        public bool AutoRelaunch = true;
        public bool AutoCheckUpdates = true;

        private static string SettingsPath
        {
            get
            {
                string dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "AntigravitySync");
                Directory.CreateDirectory(dir);
                return Path.Combine(dir, "settings.json");
            }
        }

        public static AppSettings Load()
        {
            var s = new AppSettings();
            try
            {
                if (File.Exists(SettingsPath))
                {
                    string json = File.ReadAllText(SettingsPath, Encoding.UTF8);
                    s.SyncRoot = ReadJsonString(json, "SyncRoot", s.SyncRoot);
                    s.Theme = ReadJsonString(json, "Theme", s.Theme);
                    s.AutoRelaunch = ReadJsonBool(json, "AutoRelaunch", s.AutoRelaunch);
                    s.AutoCheckUpdates = ReadJsonBool(json, "AutoCheckUpdates", s.AutoCheckUpdates);
                }
            }
            catch { }
            return s;
        }

        public void Save()
        {
            try
            {
                string json = "{\n" +
                    "  \"SyncRoot\": \"" + EscapeJson(SyncRoot) + "\",\n" +
                    "  \"Theme\": \"" + EscapeJson(Theme) + "\",\n" +
                    "  \"AutoRelaunch\": " + (AutoRelaunch ? "true" : "false") + ",\n" +
                    "  \"AutoCheckUpdates\": " + (AutoCheckUpdates ? "true" : "false") + "\n" +
                    "}\n";
                File.WriteAllText(SettingsPath, json, Encoding.UTF8);
            }
            catch { }
        }

        private static string EscapeJson(string v)
        {
            return (v ?? "").Replace("\\", "\\\\").Replace("\"", "\\\"");
        }

        private static string ReadJsonString(string json, string key, string fallback)
        {
            var m = Regex.Match(json, "\"" + Regex.Escape(key) + "\"\\s*:\\s*\"((?:\\\\.|[^\"])*)\"");
            if (!m.Success) return fallback;
            return m.Groups[1].Value.Replace("\\\"", "\"").Replace("\\\\", "\\");
        }

        private static bool ReadJsonBool(string json, string key, bool fallback)
        {
            var m = Regex.Match(json, "\"" + Regex.Escape(key) + "\"\\s*:\\s*(true|false)", RegexOptions.IgnoreCase);
            if (!m.Success) return fallback;
            return string.Equals(m.Groups[1].Value, "true", StringComparison.OrdinalIgnoreCase);
        }
    }

    public class ThemePalette
    {
        public Color Bg;
        public Color Surface;
        public Color Border;
        public Color Text;
        public Color Muted;
        public Color Green;
        public Color Cyan;
        public Color Amber;
        public Color Red;
        public Color ButtonText;

        public static ThemePalette ForName(string theme)
        {
            string resolved = theme;
            if (string.Equals(theme, "System", StringComparison.OrdinalIgnoreCase))
            {
                resolved = IsWindowsDarkMode() ? "Dark" : "Light";
            }

            if (string.Equals(resolved, "Dark Blue", StringComparison.OrdinalIgnoreCase))
            {
                return new ThemePalette
                {
                    Bg = Color.FromArgb(15, 23, 42),
                    Surface = Color.FromArgb(30, 41, 59),
                    Border = Color.FromArgb(51, 65, 85),
                    Text = Color.FromArgb(241, 245, 249),
                    Muted = Color.FromArgb(148, 163, 184),
                    Green = Color.FromArgb(34, 197, 94),
                    Cyan = Color.FromArgb(56, 189, 248),
                    Amber = Color.FromArgb(245, 158, 11),
                    Red = Color.FromArgb(239, 68, 68),
                    ButtonText = Color.White
                };
            }
            if (string.Equals(resolved, "Dark", StringComparison.OrdinalIgnoreCase))
            {
                return new ThemePalette
                {
                    Bg = Color.FromArgb(24, 24, 27),
                    Surface = Color.FromArgb(39, 39, 42),
                    Border = Color.FromArgb(63, 63, 70),
                    Text = Color.FromArgb(244, 244, 245),
                    Muted = Color.FromArgb(161, 161, 170),
                    Green = Color.FromArgb(34, 197, 94),
                    Cyan = Color.FromArgb(139, 92, 246),
                    Amber = Color.FromArgb(245, 158, 11),
                    Red = Color.FromArgb(239, 68, 68),
                    ButtonText = Color.White
                };
            }
            return new ThemePalette
            {
                Bg = Color.FromArgb(248, 250, 252),
                Surface = Color.White,
                Border = Color.FromArgb(203, 213, 225),
                Text = Color.FromArgb(15, 23, 42),
                Muted = Color.FromArgb(100, 116, 139),
                Green = Color.FromArgb(22, 163, 74),
                Cyan = Color.FromArgb(2, 132, 199),
                Amber = Color.FromArgb(217, 119, 6),
                Red = Color.FromArgb(220, 38, 38),
                ButtonText = Color.White
            };
        }

        private static bool IsWindowsDarkMode()
        {
            try
            {
                using (var key = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"))
                {
                    if (key != null)
                    {
                        object val = key.GetValue("AppsUseLightTheme");
                        if (val is int) return ((int)val) == 0;
                    }
                }
            }
            catch { }
            return false;
        }
    }

    public class SyncTarget
    {
        public string Name;
        public string LocalPath;
        public bool IsDir;
    }

    public class MainForm : Form
    {
        public const string AppVersion = "1.0.1";
        public const string RepoOwner = "Caspi7";
        public const string RepoName = "antigravity-sync";
        public static readonly string ReleasesPageUrl = "https://github.com/" + RepoOwner + "/" + RepoName + "/releases";
        public static readonly string LatestReleaseApiUrl = "https://api.github.com/repos/" + RepoOwner + "/" + RepoName + "/releases/latest";

        private readonly AppSettings _settings;
        private readonly string _autoMode;
        private readonly bool _autoRelaunchArg;
        private CancellationTokenSource _cts;
        private bool _isBusy;

        private Label _lblTitle;
        private Label _lblStatus;
        private TextBox _txtSyncRoot;
        private Button _btnBrowse;
        private ComboBox _cmbTheme;
        private CheckBox _chkRelaunch;
        private CheckBox _chkAutoUpdate;
        private Button _btnPush;
        private Button _btnPull;
        private Button _btnCancel;
        private Button _btnUpdate;
        private Button _btnReleaseNotes;
        private ProgressBar _progressBar;
        private Label _lblProgress;
        private RichTextBox _logBox;
        private ToolTip _toolTip;

        private string _latestVersion;
        private string _latestInstallerUrl;
        private string _latestSha256Url;

        public MainForm(string autoMode, bool autoRelaunchArg)
        {
            _settings = AppSettings.Load();
            _autoMode = autoMode;
            _autoRelaunchArg = autoRelaunchArg;

            Text = "Antigravity Sync v" + AppVersion;
            ClientSize = new Size(640, 470);
            MinimumSize = new Size(580, 430);
            StartPosition = FormStartPosition.CenterScreen;
            Font = new Font("Segoe UI", 9.5f);

            BuildUi();
            ApplyTheme(_settings.Theme);

            Shown += OnFormShown;
        }

        private void BuildUi()
        {
            _toolTip = new ToolTip();

            _lblTitle = new Label
            {
                Text = "Antigravity Sync v" + AppVersion,
                Font = new Font("Segoe UI Semibold", 14f),
                Location = new Point(18, 14),
                AutoSize = true
            };

            _cmbTheme = new ComboBox
            {
                DropDownStyle = ComboBoxStyle.DropDownList,
                FlatStyle = FlatStyle.Flat,
                Location = new Point(492, 16),
                Size = new Size(130, 26),
                Anchor = AnchorStyles.Top | AnchorStyles.Right
            };
            _cmbTheme.Items.AddRange(new object[] { "System", "Light", "Dark", "Dark Blue" });
            _cmbTheme.SelectedItem = _settings.Theme;
            if (_cmbTheme.SelectedIndex < 0) _cmbTheme.SelectedIndex = 0;
            _cmbTheme.SelectedIndexChanged += (s, e) =>
            {
                _settings.Theme = _cmbTheme.SelectedItem.ToString();
                _settings.Save();
                ApplyTheme(_settings.Theme);
            };
            _toolTip.SetToolTip(_cmbTheme, "Switch between System, Light, Dark, and Dark Blue themes");

            _lblStatus = new Label
            {
                Text = "Ready",
                Location = new Point(20, 46),
                Size = new Size(600, 20),
                Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right
            };

            var lblFolder = new Label
            {
                Text = "Sync Folder:",
                Location = new Point(20, 78),
                AutoSize = true
            };

            _txtSyncRoot = new TextBox
            {
                Text = _settings.SyncRoot,
                Location = new Point(108, 75),
                Size = new Size(424, 24),
                Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right
            };
            _txtSyncRoot.TextChanged += (s, e) =>
            {
                _settings.SyncRoot = _txtSyncRoot.Text.Trim();
                _settings.Save();
            };
            _toolTip.SetToolTip(_txtSyncRoot, "Cloud sync folder (e.g. G:\\My Drive\\AntigravitySync)");

            _btnBrowse = new Button
            {
                Text = "Browse...",
                Location = new Point(540, 74),
                Size = new Size(82, 26),
                FlatStyle = FlatStyle.Flat,
                Anchor = AnchorStyles.Top | AnchorStyles.Right
            };
            _btnBrowse.Click += (s, e) =>
            {
                using (var dlg = new FolderBrowserDialog())
                {
                    dlg.SelectedPath = _txtSyncRoot.Text;
                    if (dlg.ShowDialog(this) == DialogResult.OK)
                    {
                        _txtSyncRoot.Text = dlg.SelectedPath;
                    }
                }
            };
            _toolTip.SetToolTip(_btnBrowse, "Choose your Google Drive, OneDrive, or Dropbox sync folder");

            _chkRelaunch = new CheckBox
            {
                Text = "Relaunch Antigravity after sync",
                Checked = _settings.AutoRelaunch,
                Location = new Point(20, 110),
                AutoSize = true
            };
            _chkRelaunch.CheckedChanged += (s, e) =>
            {
                _settings.AutoRelaunch = _chkRelaunch.Checked;
                _settings.Save();
            };
            _toolTip.SetToolTip(_chkRelaunch, "Automatically reopen Antigravity once sync completes");

            _chkAutoUpdate = new CheckBox
            {
                Text = "Check for updates on startup",
                Checked = _settings.AutoCheckUpdates,
                Location = new Point(260, 110),
                AutoSize = true
            };
            _chkAutoUpdate.CheckedChanged += (s, e) =>
            {
                _settings.AutoCheckUpdates = _chkAutoUpdate.Checked;
                _settings.Save();
            };
            _toolTip.SetToolTip(_chkAutoUpdate, "Automatically check GitHub Releases for newer versions");

            _btnPush = new Button
            {
                Text = "Push to Drive",
                Location = new Point(20, 144),
                Size = new Size(165, 38),
                FlatStyle = FlatStyle.Flat,
                Font = new Font("Segoe UI Semibold", 10f)
            };
            _btnPush.Click += (s, e) => StartSync("Push", _chkRelaunch.Checked);
            _toolTip.SetToolTip(_btnPush, "Close Antigravity and upload local data to your sync folder");

            _btnPull = new Button
            {
                Text = "Pull from Drive",
                Location = new Point(195, 144),
                Size = new Size(165, 38),
                FlatStyle = FlatStyle.Flat,
                Font = new Font("Segoe UI Semibold", 10f)
            };
            _btnPull.Click += (s, e) => StartSync("Pull", _chkRelaunch.Checked);
            _toolTip.SetToolTip(_btnPull, "Close Antigravity, take a safety backup, and download data from your sync folder");

            _btnCancel = new Button
            {
                Text = "Cancel",
                Location = new Point(370, 144),
                Size = new Size(88, 38),
                FlatStyle = FlatStyle.Flat,
                Enabled = false
            };
            _btnCancel.Click += (s, e) =>
            {
                if (_cts != null) _cts.Cancel();
            };
            _toolTip.SetToolTip(_btnCancel, "Cancel the currently running sync operation");

            _btnReleaseNotes = new Button
            {
                Text = "Release Notes",
                Location = new Point(512, 144),
                Size = new Size(110, 38),
                FlatStyle = FlatStyle.Flat,
                Anchor = AnchorStyles.Top | AnchorStyles.Right
            };
            _btnReleaseNotes.Click += (s, e) => ShowReleaseNotes();
            _toolTip.SetToolTip(_btnReleaseNotes, "View what's new in Antigravity Sync v" + AppVersion);

            _btnUpdate = new Button
            {
                Text = "Update Available",
                Location = new Point(20, 190),
                Size = new Size(602, 30),
                FlatStyle = FlatStyle.Flat,
                Visible = false,
                Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right
            };
            _btnUpdate.Click += (s, e) => StartUpdateDownload();
            _toolTip.SetToolTip(_btnUpdate, "Download, verify SHA-256, and install the latest release");

            _progressBar = new ProgressBar
            {
                Location = new Point(20, 228),
                Size = new Size(602, 18),
                Minimum = 0,
                Maximum = 100,
                Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right
            };

            _lblProgress = new Label
            {
                Text = "0% — Idle",
                Location = new Point(20, 250),
                Size = new Size(602, 18),
                Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right
            };

            _logBox = new RichTextBox
            {
                Location = new Point(20, 274),
                Size = new Size(602, 178),
                ReadOnly = true,
                BorderStyle = BorderStyle.FixedSingle,
                Font = new Font("Consolas", 9f),
                Anchor = AnchorStyles.Top | AnchorStyles.Bottom | AnchorStyles.Left | AnchorStyles.Right
            };

            Controls.AddRange(new Control[]
            {
                _lblTitle, _cmbTheme, _lblStatus, lblFolder, _txtSyncRoot, _btnBrowse,
                _chkRelaunch, _chkAutoUpdate, _btnPush, _btnPull, _btnCancel, _btnReleaseNotes,
                _btnUpdate, _progressBar, _lblProgress, _logBox
            });
        }

        private void ApplyTheme(string themeName)
        {
            var p = ThemePalette.ForName(themeName);
            BackColor = p.Bg;
            ForeColor = p.Text;

            foreach (Control c in Controls)
            {
                if (c is Label || c is CheckBox)
                {
                    c.ForeColor = p.Text;
                    c.BackColor = p.Bg;
                }
            }

            _lblStatus.ForeColor = p.Muted;
            _lblProgress.ForeColor = p.Muted;

            _txtSyncRoot.BackColor = p.Surface;
            _txtSyncRoot.ForeColor = p.Text;
            _cmbTheme.BackColor = p.Surface;
            _cmbTheme.ForeColor = p.Text;
            _logBox.BackColor = p.Surface;
            _logBox.ForeColor = p.Text;

            StyleButton(_btnPush, p.Green, p.ButtonText, p.Green);
            StyleButton(_btnPull, p.Cyan, p.ButtonText, p.Cyan);
            StyleButton(_btnCancel, p.Red, p.ButtonText, p.Red);
            StyleButton(_btnUpdate, p.Amber, Color.Black, p.Amber);
            StyleButton(_btnBrowse, p.Surface, p.Text, p.Border);
            StyleButton(_btnReleaseNotes, p.Surface, p.Text, p.Border);
        }

        private static void StyleButton(Button b, Color bg, Color fg, Color border)
        {
            b.BackColor = bg;
            b.ForeColor = fg;
            b.FlatAppearance.BorderColor = border;
            b.FlatAppearance.BorderSize = 1;
        }

        private void OnFormShown(object sender, EventArgs e)
        {
            UpdateAntigravityStatus();
            if (!string.IsNullOrEmpty(_autoMode))
            {
                StartSync(_autoMode, _autoRelaunchArg || _chkRelaunch.Checked);
            }
            else if (_settings.AutoCheckUpdates)
            {
                Task.Run(() => CheckForUpdatesAsync());
            }
        }

        private void UpdateAntigravityStatus()
        {
            bool running = Process.GetProcessesByName("Antigravity").Length > 0;
            var p = ThemePalette.ForName(_settings.Theme);
            if (running)
            {
                _lblStatus.Text = "Antigravity is running (will be closed automatically before sync)";
                _lblStatus.ForeColor = p.Amber;
            }
            else
            {
                _lblStatus.Text = "Antigravity is closed — ready to sync";
                _lblStatus.ForeColor = p.Green;
            }
        }

        private void AppendLog(string message)
        {
            if (InvokeRequired)
            {
                BeginInvoke((Action)(() => AppendLog(message)));
                return;
            }
            _logBox.AppendText("[" + DateTime.Now.ToString("HH:mm:ss") + "] " + message + Environment.NewLine);
            _logBox.ScrollToCaret();
        }

        private void SetProgress(int percent, string text)
        {
            if (InvokeRequired)
            {
                BeginInvoke((Action)(() => SetProgress(percent, text)));
                return;
            }
            _progressBar.Value = Math.Max(0, Math.Min(100, percent));
            _lblProgress.Text = text;
        }

        private void SetBusy(bool busy)
        {
            if (InvokeRequired)
            {
                BeginInvoke((Action)(() => SetBusy(busy)));
                return;
            }
            _isBusy = busy;
            _btnPush.Enabled = !busy;
            _btnPull.Enabled = !busy;
            _btnBrowse.Enabled = !busy;
            _txtSyncRoot.Enabled = !busy;
            _btnCancel.Enabled = busy;
        }

        private List<SyncTarget> GetTargets()
        {
            string user = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            string appData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
            return new List<SyncTarget>
            {
                new SyncTarget { Name = "conversations", LocalPath = Path.Combine(user, @".gemini\antigravity\conversations"), IsDir = true },
                new SyncTarget { Name = "brain", LocalPath = Path.Combine(user, @".gemini\antigravity\brain"), IsDir = true },
                new SyncTarget { Name = "annotations", LocalPath = Path.Combine(user, @".gemini\antigravity\annotations"), IsDir = true },
                new SyncTarget { Name = "config", LocalPath = Path.Combine(user, @".gemini\config"), IsDir = true },
                new SyncTarget { Name = "globalStorage", LocalPath = Path.Combine(appData, @"Antigravity\User\globalStorage"), IsDir = true },
                new SyncTarget { Name = "workspaceStorage", LocalPath = Path.Combine(appData, @"Antigravity\User\workspaceStorage"), IsDir = true },
                new SyncTarget { Name = "agyhub_summaries", LocalPath = Path.Combine(user, @".gemini\antigravity\agyhub_summaries_proto.pb"), IsDir = false }
            };
        }

        private void StartSync(string mode, bool relaunch)
        {
            if (_isBusy) return;
            string syncRoot = _txtSyncRoot.Text.Trim();
            if (string.IsNullOrEmpty(syncRoot))
            {
                MessageBox.Show(this, "Please specify a Sync Folder first.", "Antigravity Sync", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                return;
            }

            _cts = new CancellationTokenSource();
            var token = _cts.Token;
            SetBusy(true);
            _logBox.Clear();
            AppendLog("Starting " + mode + " with " + syncRoot + "...");

            Task.Run(() =>
            {
                var sw = Stopwatch.StartNew();
                try
                {
                    CloseAntigravity(token);
                    Directory.CreateDirectory(syncRoot);
                    var targets = GetTargets();

                    if (mode == "Pull")
                    {
                        SetProgress(5, "5% — Creating safety backup...");
                        CreateBackup(targets, token);
                    }

                    int total = targets.Count;
                    for (int i = 0; i < total; i++)
                    {
                        token.ThrowIfCancellationRequested();
                        var t = targets[i];
                        string remote = Path.Combine(syncRoot, t.Name);
                        string src = mode == "Push" ? t.LocalPath : remote;
                        string dst = mode == "Push" ? remote : t.LocalPath;

                        int pct = 10 + (int)((i / (double)total) * 85);
                        string eta = i > 0 ? FormatEta(sw.Elapsed, i, total) : "calculating...";
                        SetProgress(pct, pct + "% — [" + mode + "] " + t.Name + " (ETA: " + eta + ")");

                        bool exists = t.IsDir ? Directory.Exists(src) : File.Exists(src);
                        if (!exists)
                        {
                            AppendLog("Skipping " + t.Name + " (not found)");
                            continue;
                        }

                        AppendLog("[" + mode + "] " + t.Name + "...");
                        RunRobocopy(src, dst, t.IsDir, token);
                    }

                    SetProgress(100, "100% — " + mode + " completed cleanly in " + sw.Elapsed.TotalSeconds.ToString("0.0") + "s");
                    AppendLog("Done! " + mode + " completed cleanly.");

                    if (relaunch)
                    {
                        RelaunchAntigravity();
                    }

                    if (!string.IsNullOrEmpty(_autoMode))
                    {
                        Thread.Sleep(1200);
                        BeginInvoke((Action)(() => Close()));
                    }
                }
                catch (OperationCanceledException)
                {
                    SetProgress(0, "Cancelled");
                    AppendLog("Operation cancelled by user.");
                }
                catch (Exception ex)
                {
                    SetProgress(0, "Error");
                    AppendLog("ERROR: " + ex.Message);
                    BeginInvoke((Action)(() => MessageBox.Show(this, ex.Message, "Sync Error", MessageBoxButtons.OK, MessageBoxIcon.Error)));
                }
                finally
                {
                    SetBusy(false);
                    BeginInvoke((Action)UpdateAntigravityStatus);
                }
            }, token);
        }

        private void CloseAntigravity(CancellationToken token)
        {
            var procs = Process.GetProcessesByName("Antigravity");
            if (procs.Length == 0) return;

            AppendLog("Closing Antigravity...");
            foreach (var p in procs)
            {
                try { p.Kill(); } catch { }
            }
            foreach (var p in procs)
            {
                try { p.WaitForExit(15000); } catch { }
            }
            Thread.Sleep(1000);
            token.ThrowIfCancellationRequested();
        }

        private void CreateBackup(List<SyncTarget> targets, CancellationToken token)
        {
            string user = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            string backupDir = Path.Combine(user, @".gemini\antigravity_backup_" + DateTime.Now.ToString("yyyyMMdd_HHmmss"));
            Directory.CreateDirectory(backupDir);
            AppendLog("Creating safety backup at " + backupDir + "...");

            foreach (var t in targets)
            {
                token.ThrowIfCancellationRequested();
                string dst = Path.Combine(backupDir, t.Name);
                if (t.IsDir && Directory.Exists(t.LocalPath))
                {
                    RunRobocopy(t.LocalPath, dst, true, token);
                }
                else if (!t.IsDir && File.Exists(t.LocalPath))
                {
                    File.Copy(t.LocalPath, dst, true);
                }
            }
        }

        private void RunRobocopy(string src, string dst, bool isDir, CancellationToken token)
        {
            string args;
            if (isDir)
            {
                Directory.CreateDirectory(dst);
                args = "\"" + src + "\" \"" + dst + "\" /MIR /MT:8 /R:2 /W:2 /NFL /NDL /NP /NJH /NJS";
            }
            else
            {
                string srcDir = Path.GetDirectoryName(src);
                string srcFile = Path.GetFileName(src);
                string dstDir = Path.GetDirectoryName(dst);
                Directory.CreateDirectory(dstDir);
                args = "\"" + srcDir + "\" \"" + dstDir + "\" \"" + srcFile + "\" /R:2 /W:2 /NFL /NDL /NP /NJH /NJS";
            }

            var psi = new ProcessStartInfo
            {
                FileName = "robocopy.exe",
                Arguments = args,
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true
            };

            using (var proc = Process.Start(psi))
            {
                // Drain stdout/stderr asynchronously to prevent pipe buffer deadlock on large folders
                proc.OutputDataReceived += (s, e) => { };
                proc.ErrorDataReceived += (s, e) => { };
                proc.BeginOutputReadLine();
                proc.BeginErrorReadLine();

                while (!proc.WaitForExit(200))
                {
                    if (token.IsCancellationRequested)
                    {
                        try { proc.Kill(); } catch { }
                        token.ThrowIfCancellationRequested();
                    }
                }
                proc.WaitForExit();
                if (proc.ExitCode >= 8)
                {
                    throw new Exception("robocopy failed with exit code " + proc.ExitCode);
                }
            }
        }

        private void RelaunchAntigravity()
        {
            string localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            string exe = Path.Combine(localAppData, @"Programs\antigravity\Antigravity.exe");
            if (File.Exists(exe))
            {
                AppendLog("Relaunching Antigravity...");
                Process.Start(new ProcessStartInfo { FileName = exe, UseShellExecute = true });
            }
        }

        private static string FormatEta(TimeSpan elapsed, int done, int total)
        {
            double remainingSec = (elapsed.TotalSeconds / done) * (total - done);
            if (remainingSec < 1) return "<1s";
            if (remainingSec < 60) return Math.Ceiling(remainingSec) + "s";
            return TimeSpan.FromSeconds(remainingSec).ToString(@"m\m\ s\s");
        }

        private void CheckForUpdatesAsync()
        {
            try
            {
                ServicePointManager.SecurityProtocol = SecurityProtocolType.Tls12;
                using (var wc = new WebClient())
                {
                    wc.Headers.Add("User-Agent", "AntigravitySync/" + AppVersion);
                    string json = wc.DownloadString(LatestReleaseApiUrl);

                    var tagMatch = Regex.Match(json, "\"tag_name\"\\s*:\\s*\"v?([^\"]+)\"");
                    if (!tagMatch.Success) return;
                    string latest = tagMatch.Groups[1].Value.Trim();

                    if (!IsNewerVersion(latest, AppVersion)) return;

                    var exeMatch = Regex.Match(json, "\"browser_download_url\"\\s*:\\s*\"([^\"]+Setup[^\"]*\\.exe)\"", RegexOptions.IgnoreCase);
                    var shaMatch = Regex.Match(json, "\"browser_download_url\"\\s*:\\s*\"([^\"]+SHA256SUMS\\.txt)\"", RegexOptions.IgnoreCase);
                    if (!exeMatch.Success) return;

                    _latestVersion = latest;
                    _latestInstallerUrl = exeMatch.Groups[1].Value;
                    _latestSha256Url = shaMatch.Success ? shaMatch.Groups[1].Value : null;

                    BeginInvoke((Action)(() =>
                    {
                        _btnUpdate.Text = "Update Available: v" + _latestVersion + " — Click to Download & Install";
                        _btnUpdate.Visible = true;
                    }));
                }
            }
            catch { }
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

        private void StartUpdateDownload()
        {
            if (string.IsNullOrEmpty(_latestInstallerUrl))
            {
                Process.Start(new ProcessStartInfo { FileName = ReleasesPageUrl, UseShellExecute = true });
                return;
            }

            SetBusy(true);
            _btnUpdate.Enabled = false;
            AppendLog("Downloading Antigravity Sync v" + _latestVersion + "...");

            Task.Run(() =>
            {
                try
                {
                    ServicePointManager.SecurityProtocol = SecurityProtocolType.Tls12;
                    string tempExe = Path.Combine(Path.GetTempPath(), "AntigravitySync-Setup-v" + _latestVersion + ".exe");

                    using (var wc = new WebClient())
                    {
                        wc.Headers.Add("User-Agent", "AntigravitySync/" + AppVersion);
                        wc.DownloadProgressChanged += (s, e) =>
                        {
                            SetProgress(e.ProgressPercentage, e.ProgressPercentage + "% — Downloading update...");
                        };
                        wc.DownloadFileTaskAsync(new Uri(_latestInstallerUrl), tempExe).GetAwaiter().GetResult();
                    }

                    if (!string.IsNullOrEmpty(_latestSha256Url))
                    {
                        SetProgress(95, "95% — Verifying SHA-256 checksum...");
                        using (var wc = new WebClient())
                        {
                            wc.Headers.Add("User-Agent", "AntigravitySync/" + AppVersion);
                            string sums = wc.DownloadString(_latestSha256Url);
                            string actualHash = ComputeSha256(tempExe);
                            if (sums.IndexOf(actualHash, StringComparison.OrdinalIgnoreCase) < 0)
                            {
                                throw new Exception("SHA-256 verification failed! Aborting update.");
                            }
                            AppendLog("SHA-256 verified: " + actualHash);
                        }
                    }

                    AppendLog("Launching installer silently and exiting...");
                    Program.ReleaseSingleInstanceMutex();
                    Process.Start(new ProcessStartInfo
                    {
                        FileName = tempExe,
                        Arguments = "/S",
                        UseShellExecute = true
                    });
                    BeginInvoke((Action)(() => Application.Exit()));
                }
                catch (Exception ex)
                {
                    AppendLog("Update failed: " + ex.Message);
                    BeginInvoke((Action)(() =>
                    {
                        var res = MessageBox.Show(this,
                            "Auto-update failed: " + ex.Message + "\n\nOpen the GitHub Releases page instead?",
                            "Update Failed", MessageBoxButtons.YesNo, MessageBoxIcon.Warning);
                        if (res == DialogResult.Yes)
                        {
                            Process.Start(new ProcessStartInfo { FileName = ReleasesPageUrl, UseShellExecute = true });
                        }
                        _btnUpdate.Enabled = true;
                        SetBusy(false);
                    }));
                }
            });
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

        private void ShowReleaseNotes()
        {
            MessageBox.Show(this,
                "Antigravity Sync v" + AppVersion + "\n\n" +
                "• Native Windows GUI (PE Subsystem 2) — zero console/PowerShell flash\n" +
                "• One-click Push & Pull with automatic Antigravity close/relaunch and safety backup\n" +
                "• Multi-theme Windows 11 Fluent UI (System, Light, Dark, Dark Blue)\n" +
                "• Built-in 1-click GitHub auto-updater with SHA-256 verification\n" +
                "• Desktop shortcuts for Push Antigravity & Pull Antigravity\n\n" +
                "Releases: " + ReleasesPageUrl,
                "Release Notes — v" + AppVersion,
                MessageBoxButtons.OK,
                MessageBoxIcon.Information);
        }
    }

    static class Program
    {
        private static Mutex _mutex;

        public static void ReleaseSingleInstanceMutex()
        {
            if (_mutex != null)
            {
                try { _mutex.ReleaseMutex(); } catch { }
                try { _mutex.Dispose(); } catch { }
                _mutex = null;
            }
        }

        [STAThread]
        static void Main(string[] args)
        {
            bool createdNew;
            _mutex = new Mutex(true, @"Local\AntigravitySync_SingleInstance", out createdNew);
            if (!createdNew)
            {
                MessageBox.Show("Antigravity Sync is already running.", "Antigravity Sync", MessageBoxButtons.OK, MessageBoxIcon.Information);
                return;
            }

            string autoMode = null;
            bool relaunch = false;
            foreach (string a in args)
            {
                if (string.Equals(a, "--push", StringComparison.OrdinalIgnoreCase)) autoMode = "Push";
                else if (string.Equals(a, "--pull", StringComparison.OrdinalIgnoreCase)) autoMode = "Pull";
                else if (string.Equals(a, "--relaunch", StringComparison.OrdinalIgnoreCase)) relaunch = true;
            }

            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            Application.Run(new MainForm(autoMode, relaunch));
            ReleaseSingleInstanceMutex();
        }
    }
}
