import QtQuick
import qs.Commons
import qs.Ui

Column {
  id: gpuPage

  property var devices: []
  property var processes: []
  property real totalMemoryBytes: 0
  property color foreground: Color.foreground
  property string fontFamily: Style.font.family

  spacing: Style.space(8)

  PlainText { text: "GRAPHICS"; color: gpuPage.foreground; opacity: 0.65; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.caption }
  PlainText { text: gpuPage.devices.length > 0 ? gpuPage.devices.length + " DETECTED GPU" + (gpuPage.devices.length === 1 ? "" : "S") : "NO READABLE GPUS"; color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }

  Repeater {
    model: gpuPage.devices
    delegate: Column {
      id: deviceColumn
      width: gpuPage.width
      spacing: Style.space(3)

      // processesFor() scans the whole process table; compute it once per
      // device instead of once for the Repeater and again for the empty state.
      readonly property var gpuProcesses: gpuPage.processesFor(modelData)

      Row {
        width: parent.width
        PlainText { width: parent.width - Style.space(90); text: modelData.name + " [" + modelData.vendor + "]"; color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall; elide: Text.ElideRight }
        PlainText { width: Style.space(90); text: modelData.usage_percent === null ? "--" : Number(modelData.usage_percent).toFixed(0) + "%"; color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall; horizontalAlignment: Text.AlignRight }
      }
      Rectangle { width: parent.width; height: Style.space(10); color: Qt.rgba(gpuPage.foreground.r, gpuPage.foreground.g, gpuPage.foreground.b, 0.15); Rectangle { width: parent.width * gpuPage.percent(modelData.usage_percent) / 100; height: parent.height; color: Color.accent } }
      PlainText { text: modelData.vendor === "Intel" ? "MEM shared RAM" : "VRAM " + (modelData.memory_used_bytes === null || modelData.memory_total_bytes === null ? "--" : gpuPage.formatBytes(modelData.memory_used_bytes) + " / " + gpuPage.formatBytes(modelData.memory_total_bytes)); color: gpuPage.foreground; opacity: 0.7; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall }
      PlainText { text: "GPU PROCESSES"; color: gpuPage.foreground; opacity: 0.65; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.caption }
      Row {
        width: parent.width
        spacing: Style.space(8)
        PlainText { width: parent.width - Style.space(354); text: "PROCESS"; color: gpuPage.foreground; opacity: 0.55; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.caption }
        PlainText { width: Style.space(42); text: "GPU"; color: gpuPage.foreground; opacity: 0.55; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.caption }
        PlainText { width: Style.space(42); text: "CPU"; color: gpuPage.foreground; opacity: 0.55; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.caption }
        PlainText { width: Style.space(48); text: "RAM"; color: gpuPage.foreground; opacity: 0.55; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.caption }
        PlainText { width: Style.space(190); text: "VRAM"; color: gpuPage.foreground; opacity: 0.55; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.caption }
      }
      Repeater {
        model: deviceColumn.gpuProcesses
        delegate: Row {
          width: gpuPage.width
          spacing: Style.space(8)
          PlainText { width: parent.width - Style.space(354); text: (modelData.process.user || "?") + "@" + gpuPage.processName(modelData.process.cmd, modelData.process.pid) + " [" + modelData.process.pid + "]"; color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall; elide: Text.ElideRight }
          PlainText { width: Style.space(42); text: gpuPage.percentText(modelData.gpu.gpu_percent); color: Color.accent; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall; horizontalAlignment: Text.AlignRight }
          PlainText { width: Style.space(42); text: gpuPage.percentText(modelData.process.cpu_percent); color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall; horizontalAlignment: Text.AlignRight }
          PlainText { width: Style.space(48); text: gpuPage.ramPercent(modelData.process.mem_bytes); color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall; horizontalAlignment: Text.AlignRight }
          PlainText { width: Style.space(190); text: gpuPage.formatBytes(modelData.gpu.memory_used_bytes); color: gpuPage.foreground; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall; horizontalAlignment: Text.AlignRight }
        }
      }
      PlainText { visible: deviceColumn.gpuProcesses.length === 0; text: "no active GPU processes"; color: gpuPage.foreground; opacity: 0.55; font.family: gpuPage.fontFamily; font.pixelSize: Style.font.bodySmall }
    }
  }

  function processesFor(device) {
    if (!device.processes || !gpuPage.processes) return []
    var rows = []
    for (var index = 0; index < device.processes.length; index++) {
      var gpuProcess = device.processes[index]
      var process = findProcess(gpuProcess.pid)
      if (process) rows.push({ gpu: gpuProcess, process: process })
    }
    return rows
  }

  function findProcess(pid) {
    for (var index = 0; index < gpuPage.processes.length; index++) {
      if (gpuPage.processes[index].pid === pid) return gpuPage.processes[index]
    }
    return null
  }

  function processName(command, pid) {
    var executable = String(command || "").trim().split(/\s+/)[0]
    if (!executable) return String(pid)
    var slash = executable.lastIndexOf("/")
    if (slash >= 0) executable = executable.slice(slash + 1)
    executable = executable.replace(/^["']+|["']+$/g, "")
    return executable || String(pid)
  }

  function percent(value) {
    var number = Number(value)
    return isFinite(number) ? Math.max(0, Math.min(100, number)) : 0
  }

  function formatBytes(bytes) {
    if (bytes === null || bytes === undefined) return "--"
    var value = Number(bytes)
    if (!isFinite(value)) return "--"
    if (value >= 1073741824) return (value / 1073741824).toFixed(1) + "G"
    if (value >= 1048576) return (value / 1048576).toFixed(0) + "M"
    if (value >= 1024) return (value / 1024).toFixed(0) + "K"
    return Math.round(value) + "B"
  }

  function percentText(value) {
    // Number(null) is 0, so a missing value has to be rejected before the
    // isFinite check or an unknown reads as idle.
    if (value === null || value === undefined || value === "") return "--"
    var number = Number(value)
    return isFinite(number) ? number.toFixed(0) + "%" : "--"
  }

  function ramPercent(bytes) {
    var total = Number(gpuPage.totalMemoryBytes)
    var value = Number(bytes)
    return total > 0 && isFinite(value) ? (value * 100 / total).toFixed(1) + "%" : "--"
  }
}
