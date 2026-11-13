// Orbit Start sensor helper.
//
// Reads the CPU temperature with LibreHardwareMonitorLib (no window, no tray
// icon) and writes it to a small text file that Orbit Start reads:
//
//     %ProgramData%\OrbitStart\sensors.txt
//     cpu=47.5
//     time=1731500000
//
// Reading the CPU's temperature sensor needs the PawnIO driver and
// administrator rights, so this runs as a scheduled task at sign-in
// (see install-sensors.ps1). Usage: orbit-sensors.exe [--once] [output file]
using System;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Threading;
using LibreHardwareMonitor.Hardware;

static class OrbitSensors
{
    static int Main(string[] args)
    {
        bool once = args.Contains("--once");
        string output = args.FirstOrDefault(a => !a.StartsWith("--")) ?? Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData), "OrbitStart", "sensors.txt");

        bool first;
        using (var single = new Mutex(true, @"Global\OrbitStartSensors", out first))
        {
            if (!first && !once) return 0; // already running
            try { System.Diagnostics.Process.GetCurrentProcess().PriorityClass = System.Diagnostics.ProcessPriorityClass.BelowNormal; } catch { }

            var computer = new Computer { IsCpuEnabled = true };
            computer.Open();
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(output));
                while (true)
                {
                    float? cpu = ReadCpu(computer);
                    string text = string.Format(CultureInfo.InvariantCulture, "cpu={0}\ntime={1}\n",
                        cpu.HasValue ? cpu.Value.ToString("0.0", CultureInfo.InvariantCulture) : "",
                        DateTimeOffset.UtcNow.ToUnixTimeSeconds());
                    if (once) { Console.Write(text); return cpu.HasValue ? 0 : 2; }
                    Write(output, text);
                    Thread.Sleep(2000);
                }
            }
            finally { computer.Close(); }
        }
    }

    static float? ReadCpu(Computer computer)
    {
        float? package = null, hottest = null;
        foreach (IHardware hw in computer.Hardware.Where(h => h.HardwareType == HardwareType.Cpu))
        {
            hw.Update();
            foreach (ISensor s in hw.Sensors)
            {
                if (s.SensorType != SensorType.Temperature || !s.Value.HasValue) continue;
                float v = s.Value.Value;
                if (v <= 0 || v > 125) continue;
                string n = s.Name.ToLowerInvariant();
                if (n.Contains("package") || n.Contains("tctl") || n.Contains("tdie")) package = v;
                if (n.StartsWith("core") && (!hottest.HasValue || v > hottest.Value)) hottest = v;
            }
        }
        return package ?? hottest;
    }

    static void Write(string path, string text)
    {
        try
        {
            string tmp = path + ".tmp";
            File.WriteAllText(tmp, text);
            if (File.Exists(path)) File.Replace(tmp, path, null); else File.Move(tmp, path);
        }
        catch { }
    }
}
