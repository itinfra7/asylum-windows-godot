//! Constants from asylum.h, maze.c, player.c, alien.c, bullet.c, projectile.c.
//! Names keep the C spelling (minus the leading underscore) for traceability.
#![allow(dead_code)]

// ---- asylum.h
pub const PROJ_TTL: i32 = 1 << 16;
pub const PROJ_ROCKET: i32 = 1 << 15;
pub const PROJ_FIVEWAY: i32 = 1 << 14;
pub const PROJ_SLOWSPLIT: i32 = 1 << 13;
pub const PROJ_WEIRDSPLIT: i32 = 1 << 12;
pub const ROCK_DIVIDE: i32 = 1 << 14;
pub const ROCK_REDIVIDE: i32 = 1 << 13;
pub const ROCK_BURST: i32 = 1 << 12;
pub const PROJ_EXPLO: i32 = 1 << 11;
pub const PROJ_SPLIT: i32 = 1 << 10;
pub const PROJ_ATOM: i32 = 1 << 9;

pub const BULL_TTL: i32 = 1 << 16;
pub const BULL_EXPLO: i32 = 1 << 15;
pub const BULL_ACCEL: i32 = 1 << 14;
pub const BULL_HOME: i32 = 1 << 13;
pub const BULL_SLOWHOME: i32 = 1 << 12;
pub const BULL_SPLIT: i32 = 1 << 10;

pub const XLOWLIM: i32 = 0;
pub const YLOWLIM: i32 = 15 << 8;
pub const SPEEDLIM: i32 = 15 << 8;

pub const PLAYERCHANNEL: i32 = 0;
pub const EXPLOCHANNEL: i32 = 3;
pub const FIRECHANNEL: i32 = 1;
pub const SPARECHANNEL: i32 = 2;

// alien object types
pub const EXPLO: i32 = 1;
pub const EMBER: i32 = 2;
pub const PLATBASE: i32 = 3;
pub const RISEPLAT: i32 = PLATBASE;
pub const EXPLOPLAT: i32 = PLATBASE + 1;
pub const UPDOWNPLAT: i32 = PLATBASE + 2;
pub const DOWNPLAT: i32 = PLATBASE + 3;
pub const FASTPLAT: i32 = PLATBASE + 4;
pub const FASTPLATSTOP: i32 = PLATBASE + 5;
pub const FASTPLATEXPLO: i32 = PLATBASE + 6;
pub const FASTPLATFIRE: i32 = PLATBASE + 7;
pub const FALLPLAT: i32 = PLATBASE + 8;
pub const SCOREOBJ: i32 = 12;
pub const DYINGBONUS: i32 = 13;
pub const FLYINGBONUS: i32 = 14;
pub const BOOBY: i32 = 15;
pub const DECORATION: i32 = 16;
pub const EXTENDER: i32 = 17;
pub const ALIEN1: i32 = 18;

pub const FULLPITCH: i32 = 0x2155;

// ---- maze.c (block number ranges; all `char` = u8)
pub const BONUSLOW: u8 = 16;
pub const BONUSHIGH: u8 = 31;
pub const MEGABONUSLIM: u8 = 35;
pub const NEURONLOWLIM: u8 = 40;
pub const NEURONHIGHLIM: u8 = 47;
pub const GASLOWLIM: u8 = 54;
pub const GASHIGHLIM: u8 = 55;
pub const ANIMLOWLIM: u8 = 54;
pub const ANIMHIGHLIM: u8 = 63;
pub const ELECLOWLIM: u8 = 56;
pub const ELECVERTLOWLIM: u8 = 60;
pub const ELECHIGHLIM: u8 = 63;
pub const FUELAIRNO: u8 = 68;
pub const BOMBLOWLIM: u8 = 64;
pub const BOMBHIGHLIM: u8 = 79;
pub const ATOMLOWLIM: u8 = 72;
pub const ATOMHIGHLIM: u8 = 75;
pub const BOOBYLOWLIM: u8 = 76;
pub const BOOBYHIGHLIM: u8 = 79;
pub const TARGETLOWLIM: u8 = 80;
pub const POWERTARGET: u8 = 84;
pub const NUTTERTARGET: u8 = 88;
pub const TARGETHIGHLIM: u8 = 95;
pub const WEAPLOWLIM: u8 = 96;
pub const WEAPHIGHLIM: u8 = 111;
pub const SOLIDLOWLIM: u8 = 128;
pub const SOLIDHIGHLIM: u8 = 131;
pub const CRUMBLELOWLIM: u8 = 160;
pub const CRUMBLEHIGHLIM: u8 = 167;
pub const CRUMBLESTANDLOWLIM: u8 = 168;
pub const CRUMBLESTANDHIGHLIM: u8 = 175;
pub const STARSNO: u8 = 15;
pub const MARKERNO: u8 = 255;
pub const TRIGGERLIM: u8 = 193;
pub const BOXWIDTH: i32 = 20;
pub const BOXHEIGHT: i32 = 16;
pub const TARGETSCORE: i32 = 500;
pub const BOMBLOSS: i32 = 4 << 8;
pub const ATOMLOSS: i32 = 32 << 8;
pub const WINDSPEED: i32 = 1 << 8;
pub const FUELAIRDURATION: i32 = 0x18;
pub const FUELTABLIM: usize = 0x3c0;

// ---- player.c
pub const TRANSLOWLIM: u8 = 54;
pub const TRANSHIGHLIM: u8 = 63;
pub const WEAPMPMGBLAM: u8 = 102;
pub const WEAPROCKETBLAM: u8 = 111;
pub const BLIM: u8 = 112;
pub const TELEPLOWLIM: u8 = 192;
pub const TELEPHIGHLIM: u8 = 199;
pub const EXTENDNO: u8 = 152;
pub const SHUTDOWNNO: u8 = 200;
pub const NEURONSTOGET: i32 = 8;
pub const ROCKETTABLEN: i32 = 192;
pub const MPMGBLAMNO: u8 = 8;
pub const ROCKETBLAMNO: u8 = 16;
pub const FUNNYFACESPRBASE: u8 = 66;
pub const SCORESPRBASE: i32 = 16;
pub const SKELSPRITENO: u8 = 46;
pub const STARSSPRBASE: u8 = 58;
pub const DEADSPRBASE: i32 = 70;
pub const ROCKBASE: u8 = 9;
pub const MPMGBASE: u8 = 1;
pub const PLHEIGHT: i32 = 32 << 8;
pub const PLWIDTH: i32 = 16 << 8;
pub const STRENGTHINIT: i32 = 108 << 8;
pub const STRENGTHMAX: i32 = 108 << 8;
pub const SAVEVALID: i32 = 0x4b4f6349;

// ---- alien.c
pub const FIRERANGE: i32 = 128 << 8;
pub const SLEEPRANGE: i32 = 512 << 8;
pub const EXPLOCONTLOSS: i32 = 1 << 7;
pub const ALNO: usize = 0x200;
pub const BULSPRITEBASE: i32 = 16;
pub const EXPWIDTH: i32 = 24 << 8;
pub const EXPHEIGHT: i32 = 24 << 8;
pub const ALHEIGHT: i32 = 32 << 8;
pub const ALWIDTH: i32 = 16 << 8;
pub const PLATBLIM: u8 = 64;
pub const XOFS: i32 = 159;
pub const YOFS: i32 = 96;
pub const PLATNO: u8 = 176;
pub const PLATLOWLIM: u8 = 176;
pub const PLATHIGHLIM: u8 = 190;
pub const ALPLATHIGHLIM: u8 = 177;
pub const XBLOCKS_DISTURB: i32 = 18;
pub const YBLOCKS_DISTURB: i32 = 12;
pub const COLCHLIM: usize = 0x400;
pub const BULCOLCHLIM: usize = 0x400;
/// `_savearealen` = 0x9c0/32 = 78 entries of 28 bytes.
pub const SAVEAREALEN: usize = 0x9c0 / 32;

// ---- bullet.c / projectile.c
pub const BULNO: usize = 64;
pub const PROJNO: usize = 32;
pub const PROJSMALLNO: i32 = 54;
pub const ROCKETSPRITENO: i32 = 32;
pub const ROCKETSPRITELIM: i32 = 41;
pub const BULLETLOSS: i32 = 2 << 8;
pub const SPCRUMBLELOWLIM: u8 = 140;
pub const SPCRUMBLEHIGHLIM: u8 = 143;

// ---- vdu.c
pub const TEXTNO: usize = 32;
pub const CHARWIDTH: i32 = 16;

/// Size of one alien/player save store (`30 + 78*28`).
pub const STORE_LEN: usize = 30 + SAVEAREALEN * 28;
