// Flags drawn the way the map is: a 6x6 block of squares. Each entry describes the pattern;
// letters are palette keys. Forms:
//   h:abc   horizontal stripes (2 or 3)      v:abc   vertical stripes (2 or 3)
//   +:ab    Nordic cross (field a, cross b)  o:ab    field a with a 2x2 centre of b
//   six rows of six letters separated by "/" for everything else
export const palette = {
  r: '#e0484d', w: 'var(--flag-light)', b: '#3d7fe6', n: '#27408f', k: 'var(--flag-dark)', y: '#f0c648',
  g: '#2fa565', o: '#ee8a3c', c: '#62bdf0', m: '#96303a',
};

const spec = {
  DE: 'h:kry', NL: 'h:rwb', RU: 'h:wbr', AT: 'h:rwr', HU: 'h:rwg', BG: 'h:wgr', EE: 'h:bkw', LT: 'h:ygr',
  LU: 'h:rwc', AM: 'h:rbo', PL: 'h:wr', UA: 'h:by', ID: 'h:rw', MC: 'h:rw', EG: 'h:rwk', YE: 'h:rwk',
  IN: 'h:owg', UZ: 'h:cwg', AZ: 'h:crg', TJ: 'h:rwg', RS: 'h:rbw', SK: 'h:wbr', SI: 'h:wbr', HR: 'h:rwb',
  LV: 'h:mwm', LI: 'h:nr', SG: 'h:rw', AR: 'h:cwc', IR: 'h:gwr', BO: 'h:ryg', GA: 'h:gyb',
  FR: 'v:bwr', IT: 'v:gwr', IE: 'v:gwo', BE: 'v:kyr', RO: 'v:byr', MD: 'v:byr', MX: 'v:gwr', NG: 'v:gwg',
  MT: 'v:wr', PE: 'v:rwr', AD: 'v:byr',
  SE: '+:by', FI: '+:wb', DK: '+:rw', NO: '+:rn', IS: '+:br',
  JP: 'o:wr', CH: 'rrrrrr/rrwwrr/rwwwwr/rwwwwr/rrwwrr/rrrrrr', VN: 'o:ry', HK: 'o:rw', KG: 'o:ry', KZ: 'o:cy',
  AL: 'o:rk', MA: 'o:rg', TR: 'rrrrrr/rwwrrr/rwrrwr/rwrrwr/rwwrrr/rrrrrr', CY: 'o:wo', BD: 'o:gr',
  GB: 'wnrrnw/nnrrnn/rrrrrr/rrrrrr/nnrrnn/wnrrnw',
  US: 'nnnrrr/nnnwww/nnnrrr/wwwwww/rrrrrr/wwwwww',
  CA: 'rwwwwr/rwwwwr/rwrrwr/rwrrwr/rwwwwr/rwwwwr',
  KR: 'wwwwww/wkwwkw/wwrrww/wwbbww/wkwwkw/wwwwww',
  AE: 'rrgggg/rrgggg/rrwwww/rrwwww/rrkkkk/rrkkkk',
  IL: 'wwwwww/bbbbbb/wwbbww/wwbbww/bbbbbb/wwwwww',
  AU: 'rwnnnn/wrnnwn/nnnnnn/nnnwnn/nwnnnw/nnnnnn',
  NZ: 'rwnnnn/wrnnrn/nnnnnn/nnnrnn/nnnnnr/nnnnnn',
  BR: 'gggggg/ggyygg/gybbyg/gybbyg/ggyygg/gggggg',
  CZ: 'bwwwww/bbwwww/bbbwww/bbbrrr/bbrrrr/brrrrr',
  PT: 'ggrrrr/ggrrrr/gyyrrr/gyyrrr/ggrrrr/ggrrrr',
  ES: 'rrrrrr/yyyyyy/yyyyyy/yyyyyy/yyyyyy/rrrrrr',
  GR: 'bwbwww/wwwbbb/bwbwww/bbbbbb/wwwwww/bbbbbb',
  GE: 'wwrrww/wwrrww/rrrrrr/rrrrrr/wwrrww/wwrrww',
  CN: 'ryrrrr/yrrrrr/rrrrrr/rrrrrr/rrrrrr/rrrrrr',
  TW: 'nnnrrr/nwnrrr/nnnrrr/rrrrrr/rrrrrr/rrrrrr',
  TH: 'rrrrrr/wwwwww/nnnnnn/nnnnnn/wwwwww/rrrrrr',
  CL: 'nnwwww/nwwwww/nnwwww/rrrrrr/rrrrrr/rrrrrr',
  CO: 'yyyyyy/yyyyyy/yyyyyy/nnnnnn/rrrrrr/rrrrrr',
  BY: 'wrrrrr/rrrrrr/wrrrrr/rrrrrr/wggggg/rggggg',
  ZA: 'grrrrr/ygwwww/kygggg/kygggg/ygwwww/gbbbbb',
  SA: 'gggggg/gwwwwg/gggggg/gggggg/ggwwgg/gggggg',
  SO: 'cccccc/cccccc/ccwwcc/ccwwcc/cccccc/cccccc',
  EU: 'nnyynn/nynnyn/ynnnny/ynnnny/nynnyn/nnyynn',
  // ---- the rest of the world (added in bulk)
  TD: 'v:nyr',
  AF: 'v:krg', AG: 'rrkkrr/rykkyr/rrbbrr/rwwwwr/rrrrrr/rrrrrr', AO: 'rrrrrr/rrrrrr/ryyrrr/kkyykk/kkkkkk/kkkkkk',
  AQ: 'bbbbbb/bbwwbb/bwwwwb/bwwwwb/bbwwbb/bbbbbb', AS: 'rnnnnn/rwnnnn/rwwnnn/rwwwnn/rwwnnn/rrnnnn', AW: 'cccccc/cccccc/cccccc/yyyyyy/cccccc/yyyyyy',
  AX: 'bbyybb/bbyybb/yrrrry/yrrrry/bbyybb/bbyybb', BA: 'nnnnyy/nnnyyn/nnyynn/nyynnn/yynnnn/ynnnnn', BB: 'v:nyn',
  BF: 'rrrrrr/rryyrr/rryyrr/gggggg/gggggg/gggggg', BH: 'wwrrrr/wwrrrr/wwrrrr/wwrrrr/wwrrrr/wwrrrr', BI: 'rrrrrr/gwrrwg/ggwwgg/ggwwgg/gwrrwg/rrrrrr',
  BJ: 'ggyyyy/ggyyyy/ggyyyy/ggrrrr/ggrrrr/ggrrrr', BN: 'yyyyyw/yyyywk/yyywky/yywkyy/ywkyyy/wkyyyy', BS: 'kccccc/kkcccc/kkkyyy/kkkyyy/kkcccc/kccccc',
  BT: 'yyyyyo/yyyyoo/yyywoo/yywwoo/yooooo/oooooo', BW: 'cccccc/wwwwww/kkkkkk/kkkkkk/wwwwww/cccccc', BZ: 'rrrrrr/nnwwnn/nwwwwn/nwwwwn/nnwwnn/rrrrrr',
  CC: 'gggggg/gyyyyg/gyggyg/gyyyyg/gggggg/gggggg', CD: 'cccccy/ccccyr/cccyrc/ccyrcc/cyrccc/yrcccc', CF: 'bbrrbb/bbrrbb/wwrrww/ggrrgg/yyrryy/yyrryy',
  CG: 'ggggyr/gggyyr/ggyyrr/gyyrrr/yyrrrr/yrrrrr', CI: 'v:owg', CM: 'v:gry',
  CR: 'nnnnnn/wwwwww/rrrrrr/rrrrrr/wwwwww/nnnnnn', CU: 'rnnnnn/rrwwww/rrnnnn/rrnnnn/rrwwww/rnnnnn', CV: 'nnnnnn/nnnnnn/wwwwww/rrrrrr/wwwwww/nnnnnn',
  CW: 'nnnnnn/nnnnnn/nnnnnn/nnnnnn/yyyyyy/nnnnnn', CX: 'bbbbbg/bbbbgg/bbbggg/bbgggg/bggggg/gggggg', DJ: 'cccccc/wwcccc/wrwggg/wrwggg/wwgggg/gggggg',
  DM: '+:gy', DO: 'nnwwrr/nnwwrr/wwwwww/wwwwww/rrwwnn/rrwwnn', DZ: 'gggwww/gggwww/ggrwww/ggrwww/gggwww/gggwww',
  EC: 'yyyyyy/yyyyyy/yyyyyy/nnnnnn/rrrrrr/rrrrrr', EH: 'rkkkkk/rrkkkk/rrwwww/rrwwww/rrgggg/rggggg', ER: 'gggggg/rrgggg/rrrrrr/rrrrrr/rrbbbb/bbbbbb',
  ET: 'h:gyr', FJ: 'wrwccc/rrrccc/wrwccc/cccccc/cccccc/cccccc', FM: 'cccccc/cccwcc/ccwcwc/cccwcc/cccccc/cccccc',
  FO: '+:wr', GD: 'rrrrrr/ryyggr/ryyggr/rggyyr/rggyyr/rrrrrr', GG: '+:wr',
  GH: 'h:ryg', GI: 'wwwwww/wrrrrw/wrwwrw/rrrrrr/rrrrrr/rrrrrr', GL: 'wwwwww/wwwwww/wwrrww/rrwwrr/rrrrrr/rrrrrr',
  GM: 'rrrrrr/wwwwww/bbbbbb/bbbbbb/wwwwww/gggggg', GN: 'v:ryg', GQ: 'bggggg/bbgggg/bbwwww/bbwwww/bbrrrr/brrrrr',
  GT: 'v:cwc', GU: 'rrrrrr/rnnnnr/rnwwnr/rnwwnr/rnnnnr/rrrrrr', GW: 'rryyyy/rryyyy/rryyyy/rrgggg/rrgggg/rrgggg',
  GY: 'kggggg/kkgggg/kryyyg/kryyyg/kkgggg/kggggg', HN: 'h:bwb', HT: 'h:nr',
  IM: 'rrrrrr/rwrrwr/rrwwrr/rrwwrr/rwrrwr/rrrrrr', IQ: 'h:rwk', JE: 'rwwwwr/wrwwrw/wwrrww/wwrrww/wrwwrw/rwwwwr',
  JM: 'gyyyyg/kgyygk/kkggkk/kkggkk/kgyygk/gyyyyg', JO: 'rkkkkk/rrkkkk/rrwwww/rrwwww/rrgggg/rggggg', KE: 'kkkkkk/wwwwww/rrrrrr/rrrrrr/wwwwww/gggggg',
  KH: 'nnnnnn/rrrrrr/rrwwrr/rrwwrr/rrrrrr/nnnnnn', KI: 'rrrrrr/rryyrr/rrrrrr/nwnwnw/wnwnwn/nwnwnw', KM: 'gyyyyy/ggyyyy/ggwwww/ggrrrr/ggnnnn/gnnnnn',
  KN: 'ggggkr/gggkkr/ggkkrr/gkkrrr/kkrrrr/krrrrr', KP: 'bbbbbb/wwwwww/rwwrrr/rwwrrr/wwwwww/bbbbbb', KW: 'kggggg/kkgggg/kkwwww/kkwwww/kkrrrr/krrrrr',
  LA: 'rrrrrr/nnnnnn/nnwwnn/nnwwnn/nnnnnn/rrrrrr', LB: 'rrrrrr/wwwwww/wwggww/wwggww/wwwwww/rrrrrr', LC: 'cccccc/ccyycc/cwwwwc/ckkkkc/cckkcc/cccccc',
  LK: 'goommm/goommm/goommm/goommm/goommm/goommm', LR: 'nnrrrr/nnwwww/rrrrrr/wwwwww/rrrrrr/wwwwww', LS: 'bbbbbb/bbbbbb/wwwwww/wwkkww/gggggg/gggggg',
  LY: 'rrrrrr/kkkkkk/kkwwkk/kkwwkk/kkkkkk/gggggg', ME: 'yyyyyy/yrrrry/yrryry/yrrrry/yrrrry/yyyyyy', MG: 'wwrrrr/wwrrrr/wwrrrr/wwgggg/wwgggg/wwgggg',
  MH: 'nnnnno/nwnnoo/nnnoow/nnoowo/noowon/oowonn', MK: 'ryrryr/rryyrr/yyyyyy/yyyyyy/rryyrr/ryrryr', ML: 'v:gyr',
  MM: 'h:ygr', MN: 'rrbbrr/rrbbrr/ryrbbr/rrbbrr/rrbbrr/rrbbrr', MO: 'gggggg/ggwwgg/gwwwwg/ggwwgg/gggggg/gggggg',
  MP: 'nnnnnn/nnwwnn/nwwwwn/nnwwnn/nnnnnn/nnnnnn', MR: 'rrrrrr/gggggg/ggyygg/ggyygg/gggggg/rrrrrr', MU: 'rrrrrr/nnnnnn/nnnnnn/yyyyyy/yyyyyy/gggggg',
  MV: 'rrrrrr/rggggr/rgwwgr/rgwwgr/rggggr/rrrrrr', MW: 'kkkkkk/kyyykk/rrrrrr/rrrrrr/gggggg/gggggg', MY: 'nnnrrr/nynwww/rrrrrr/wwwwww/rrrrrr/wwwwww',
  MZ: 'rggggg/rrwwww/rrkkkk/rrkkkk/rrwwww/ryyyyy', NA: 'bbbbrg/bbbrrg/bbrrgg/brrggg/rrgggg/rggggg', NC: 'bbbbbb/bbbbbb/rryyrr/rryyrr/gggggg/gggggg',
  NE: 'h:owg', NF: 'v:gwg', NI: 'h:bwb',
  NP: 'nnnnnn/nrnnnn/nrrnnn/nrrrnn/nrrrrn/nnnnnn', NR: 'nnnnnn/nnnnnn/yyyyyy/nnnnnn/nwnnnn/nnnnnn', NU: 'wrwyyy/rrryyy/wrwyyy/yyyyyy/yyyyyy/yyyyyy',
  OM: 'rrwwww/rrwwww/rrrrrr/rrrrrr/rrgggg/rrgggg', PA: 'wwwrrr/wnwrrr/wwwrrr/nnnwww/nnnwrw/nnnwww', PF: 'rrrrrr/wwwwww/wbbbbw/wbbbbw/wwwwww/rrrrrr',
  PG: 'krrrrr/kkrrrr/kkkrrr/kkkkrr/kkkkkr/kkkkkk', PH: 'wbbbbb/wwbbbb/wwwbbb/wwwrrr/wwrrrr/wrrrrr', PK: 'wwgggg/wwgwgg/wwgwwg/wwgwgg/wwgggg/wwgggg',
  PR: 'nrrrrr/nnwwww/nnnrrr/nnnwww/nnrrrr/nrrrrr', PS: 'rkkkkk/rrkkkk/rrwwww/rrwwww/rrgggg/rggggg', PW: 'cccccc/cccccc/cyyccc/cyyccc/cccccc/cccccc',
  PY: 'h:rwb', QA: 'wmmmmm/wwmmmm/wmmmmm/wwmmmm/wmmmmm/wwmmmm', RW: 'cccccc/cccccc/cccccc/yyyyyy/gggggg/gggggg',
  SB: 'bbbbyg/bbbyyg/bbyygg/byyggg/yygggg/yggggg', SC: 'bbbbyr/bbbyrr/bbyrrw/byrrww/yrrwwg/rrwwgg', SD: 'grrrrr/ggrrrr/ggwwww/ggwwww/ggkkkk/gkkkkk',
  SL: 'h:gwb', SM: 'h:wc', SN: 'v:gyr',
  SR: 'gggggg/wwwwww/rrrrrr/rrrrrr/wwwwww/gggggg', SS: 'bkkkkk/bbwwww/bbrrrr/bbrrrr/bbwwww/bggggg', ST: 'rggggg/rrgggg/rryyyy/rryyyy/rrgggg/rggggg',
  SV: 'h:nwn', SX: 'wrrrrr/wwrrrr/wwwnnn/wwwnnn/wwnnnn/wnnnnn', SY: 'h:rwk',
  SZ: 'nnnnnn/yyyyyy/mmmmmm/mmmmmm/yyyyyy/nnnnnn', TF: 'bwrnnn/bwrnnn/bwrnnn/nnnnnn/nnwnwn/nnnnnn', TG: 'rrgggg/rryyyy/gggggg/yyyyyy/gggggg/yyyyyy',
  TK: 'nnnnnn/nnnnnn/nyynnn/nnyyyn/nnnnnn/nnnnnn', TL: 'yrrrrr/yyrrrr/yykrrr/yykkrr/yyrrrr/yrrrrr', TM: 'rrgggg/rrgggg/rrgggg/rrgggg/rrgggg/rrgggg',
  TN: 'rrrrrr/rrwwrr/rwrrwr/rwrrwr/rrwwrr/rrrrrr', TO: 'wrwrrr/rrrrrr/wrwrrr/rrrrrr/rrrrrr/rrrrrr', TT: 'wkkrrr/rwkkrr/rrwkkr/rrrwkk/rrrrwk/rrrrrw',
  TV: 'wrwccc/rrrccc/wrwccc/cyccyc/cccccc/ccyccc', TZ: 'ggggkb/gggkkb/ggkkbb/gkkbbb/kkbbbb/kbbbbb', UG: 'kkkkkk/yyyyyy/rrrrrr/kkkkkk/yyyyyy/rrrrrr',
  UY: 'ywwwww/wwwwww/bbbbbb/wwwwww/bbbbbb/wwwwww', VA: 'v:yw', VC: 'v:byg',
  VE: 'h:ybr', VI: 'wwwwww/wbyybw/wbyybw/wwyyww/wwwwww/wwwwww', VU: 'krrrrr/kkrrrr/kyyrrr/kyyggg/kkgggg/kggggg',
  WS: 'nnnrrr/nwnrrr/nnnrrr/rrrrrr/rrrrrr/rrrrrr', XK: 'nnnnnn/nwnwnw/nnyyyn/nnyyyn/nnnnnn/nnnnnn', ZM: 'gggggg/gggggg/gggggg/gggrko/gggrko/gggrko',
  ZW: 'wggggg/wwyyyy/wwrrrr/wwkkkk/wwrrrr/wyyyyy',
};
spec.BL = spec.FR;
spec.MF = spec.FR;
spec.GP = spec.FR;
spec.MQ = spec.FR;
spec.GF = spec.FR;
spec.RE = spec.FR;
spec.YT = spec.FR;
spec.PM = spec.FR;
spec.WF = spec.FR;
spec.BQ = spec.NL;
spec.BV = spec.NO;
spec.SJ = spec.NO;
spec.HM = spec.AU;
spec.UM = spec.US;
const ensign = 'nrnnnn/rrrnnn/nrnnnn/nnnnnn/nnnnnn/nnnnnn';
spec.AI = ensign;
spec.BM = ensign;
spec.KY = ensign;
spec.MS = ensign;
spec.TC = ensign;
spec.FK = ensign;
spec.PN = ensign;
spec.SH = ensign;
spec.GS = ensign;
spec.IO = ensign;
spec.VG = ensign;
spec.CK = ensign;
spec.UK = spec.GB;

function expand(s) {
  const [kind, arg] = s.includes(':') ? s.split(':') : ['x', s];
  if (kind === 'x') return arg.split('/');
  if (kind === 'h') {
    const rows = arg.length === 2 ? [0, 0, 0, 1, 1, 1] : [0, 0, 1, 1, 2, 2];
    return rows.map((i) => arg[i].repeat(6));
  }
  if (kind === 'v') {
    const cols = arg.length === 2 ? [0, 0, 0, 1, 1, 1] : [0, 0, 1, 1, 2, 2];
    return Array(6).fill(cols.map((i) => arg[i]).join(''));
  }
  const [a, b] = arg;
  if (kind === '+') return [0, 1, 2, 3, 4, 5].map((r) => (r === 2 || r === 3 ? b.repeat(6) : a + b + b + a + a + a));
  return [0, 1, 2, 3, 4, 5].map((r) => (r === 2 || r === 3 ? a + a + b + b + a + a : a.repeat(6)));
}

const cache = new Map();

/** Six rows of six palette letters for a country code, or null when there is no drawing. */
export function flagRows(cc) {
  const code = (cc || '').toUpperCase();
  if (!spec[code]) return null;
  if (!cache.has(code)) cache.set(code, expand(spec[code]));
  return cache.get(code);
}

const regionNames = (() => {
  try {
    return new Intl.DisplayNames(['ru'], { type: 'region' });
  } catch {
    return null;
  }
})();

export function countryName(cc) {
  const code = (cc || '').toUpperCase();
  if (!code) return '';
  if (code === 'EU') return 'Европа';
  try {
    return regionNames?.of(code) ?? code;
  } catch {
    return code;
  }
}
