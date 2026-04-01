// Theme dispatch
function getTheme() {
  return (typeof currentTheme !== 'undefined' && currentTheme === 'factorio') ? factorio : schematic;
}
