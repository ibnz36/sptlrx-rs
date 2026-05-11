/// Una línea de letra sincronizada en formato LRC.
#[derive(Debug, Clone)]
pub struct LrcLine {
    pub timestamp_ms: u64,
    pub text: String,
}

/// Letra mock de "Bohemian Rhapsody" — Queen.
/// Mantenida como referencia y fallback de desarrollo (Fase 1).
#[allow(dead_code)]
pub const MOCK_LRC: &str = r#"
[00:00.00] ♪ Intro instrumental ♪
[00:05.50] Is this the real life?
[00:08.80] Is this just fantasy?
[00:12.20] Caught in a landslide,
[00:14.90] No escape from reality
[00:19.20] Open your eyes,
[00:21.50] Look up to the skies and see,
[00:27.00] I'm just a poor boy, I need no sympathy,
[00:32.60] Because I'm easy come, easy go,
[00:35.60] Little high, little low,
[00:38.90] Any way the wind blows doesn't really matter to me
[00:49.10] Mama, just killed a man,
[00:52.50] Put a gun against his head,
[00:55.40] Pulled my trigger, now he's dead
[00:59.00] Mama, life had just begun,
[01:02.50] But now I've gone and thrown it all away
[01:08.50] Mama, ooh,
[01:11.50] Didn't mean to make you cry,
[01:15.10] If I'm not back again this time tomorrow,
[01:21.50] Carry on, carry on as if nothing really matters
[01:30.00] Too late, my time has come,
[01:33.50] Sends shivers down my spine,
[01:36.90] Body's aching all the time
[01:40.40] Goodbye, everybody, I've got to go,
[01:44.00] Gotta leave you all behind and face the truth
[01:49.90] Mama, ooh, (any way the wind blows)
[01:53.50] I don't want to die,
[01:57.00] I sometimes wish I'd never been born at all
[02:08.00] ♪ Guitar solo ♪
[02:38.00] I see a little silhouetto of a man,
[02:41.20] Scaramouche, scaramouche, will you do the fandango?
[02:44.80] Thunderbolt and lightning, very very frightening me
[02:49.50] Galileo, Galileo
[02:51.50] Galileo, Galileo
[02:53.50] Galileo Figaro — magnifico
[02:57.20] I'm just a poor boy, nobody loves me
[03:00.00] He's just a poor boy from a poor family,
[03:03.30] Spare him his life from this monstrosity
[03:07.00] Easy come, easy go, will you let me go?
[03:10.00] Bismillah! No, we will not let you go
[03:13.00] Let him go! Bismillah! We will not let you go
[03:16.00] Let him go! Bismillah! We will not let you go
[03:18.50] Will not let you go, let me go
[03:21.00] Never, never, never, never let me go
[03:24.00] No, no, no, no, no, no, no
[03:26.50] Oh mama mia, mama mia
[03:29.00] Mama mia, let me go
[03:31.80] Beelzebub has a devil put aside for me, for me, for me!
[03:40.00] ♪ Heavy section ♪
[03:42.00] So you think you can stone me and spit in my eye?
[03:46.00] So you think you can love me and leave me to die?
[03:50.00] Oh, baby, can't do this to me, baby,
[03:53.80] Just gotta get out, just gotta get right outta here
[04:06.00] ♪ Coda ♪
[04:23.00] Nothing really matters,
[04:27.00] Anyone can see,
[04:31.00] Nothing really matters,
[04:35.00] Nothing really matters to me
[04:44.00] Any way the wind blows...
"#;

/// Parsea texto LRC en una lista de líneas sincronizadas ordenadas por tiempo.
/// Port directo de `parse_lrc()` del PoC en Python.
pub fn parse_lrc(text: &str) -> Vec<LrcLine> {
    let mut result: Vec<LrcLine> = text
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with('[') {
                return None;
            }
            // Buscar el cierre del timestamp: [mm:ss.xx]
            let close = line.find(']')?;
            let time_str = &line[1..close];
            let lyric_text = line[close + 1..].trim();

            // Parsear "mm:ss.xx" o "mm:ss.xxx"
            let colon = time_str.find(':')?;
            let minutes: u64 = time_str[..colon].parse().ok()?;
            let seconds: f64 = time_str[colon + 1..].parse().ok()?;

            let ms = minutes * 60 * 1000 + (seconds * 1000.0) as u64;
            Some(LrcLine {
                timestamp_ms: ms,
                text: lyric_text.to_string(),
            })
        })
        .collect();

    result.sort_by_key(|l| l.timestamp_ms);
    result
}

/// Devuelve el índice de la línea activa dado el tiempo actual en milisegundos.
/// Equivale al bucle `for i, (timestamp_ms, text) in enumerate(...)` del PoC Python.
pub fn find_current_line(lyrics: &[LrcLine], pos_ms: u64) -> Option<usize> {
    if lyrics.is_empty() {
        return None;
    }
    let mut current = None;
    for (i, line) in lyrics.iter().enumerate() {
        if pos_ms >= line.timestamp_ms {
            current = Some(i);
        } else {
            break;
        }
    }
    current
}
