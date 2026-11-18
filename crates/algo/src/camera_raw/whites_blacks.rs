//! Whites and Blacks as Adobe Camera Raw (process version 2012) applies them.
//!
//! Measured as a black box on the Camera Raw Filter of Photoshop 2026 (Camera Raw 18.6), on a
//! 256-level grey ramp, an architectural render and an aerial photograph; no Adobe code or data
//! was used. Both are global curves of the grey level, applied after Shadows and Highlights,
//! Whites first:
//!
//! - **Blacks** (both ways) and **Whites down** are fixed curves, the same on every image: the
//!   ramp's rows (every 25) match the photographs' curves. Blacks down clips the darkest levels and
//!   acts on each channel (hue kept, in linear ProPhoto, like Exposure); the compressing ways
//!   (Blacks up, Whites down) scale the pixel by the change of its luminance.
//! - **Whites up** adapts to the image's white point (the 99th percentile of the log
//!   luminance): measured in stops below it, every image's move has the same shape (largest about
//!   2.5 stops below the white point, none at it and none 9 stops below), and its size grows
//!   with how far the white point lies below white: +100 lifts that level about 0.6 stops on a
//!   render that reaches white, 1.9 on photographs whose brightest levels stop half a stop short.
//!   The slider is far from linear (+25 moves a tenth of +100). It acts on each channel, hue
//!   kept, and clips at white. Fitted to 17 photographs and renders: mean |ΔL*| 0.10 (+25),
//!   0.33 (+50), 1.27 (+100) on images left out of the fit, one at a time (the previous curve of the
//!   median-to-99th-percentile distance: 6.3 at +100, which lifted dark photographs with a
//!   clipped sun far too much).
//! - Against Photoshop (mean ΔE76): render Whites 0.2–1.4, Blacks 0.3–0.9; aerial photograph
//!   Whites 0.7–1.7, Blacks 0.7–1.1 (the previous curves, measured on Photoshop 25.4 and adapted
//!   to the black and white points: up to 5.3 and 3.6 on the render).

use super::Encoding;
pub use super::contrast::Tones;
use super::exposure::{STEPS, interpolate, pixel, primaries};
use crate::photo_util::par_rows;

/// The image's tones (see [`Tones`]) that Contrast adapts to, measured before it, and that Whites
/// up adapts to (the 99th percentile is its white point), measured after Contrast.
pub type TonePoints = [Tones; 2];

/// Whites up at +25, +50 and +100: the move in stops at `x` stops from the white point (knots
/// at −10, −9.5, …, 1) is `k0(x) + k1(x)·g + k2(x)·g² + k3(x)·m/10`, with `g` how far the white
/// point lies below white and `m` the median below the white point, both in stops. Fitted by
/// least squares to Photoshop's results on 17 photographs and renders (see the module notes).
const WHITES_UP_AT: [f32; 3] = [25.0, 50.0, 100.0];
const WHITES_KNOTS: usize = 23;
#[rustfmt::skip]
static WHITES_UP: [[[f32; WHITES_KNOTS]; 4]; 3] = [
    // +25
    [
        [-0.067663, -0.0426, -0.022437, -0.0099693, -0.0033243, 0.00074127, 0.0020649, 0.002303, 0.0026923, 0.0037433, 0.0054555, 0.0082432, 0.018793, 0.027818, 0.034415, 0.044006, 0.052443, 0.061222, 0.066414, 0.06409, 0.024484, -0.055503, -0.13549],
        [-0.10082, -0.083797, -0.067444, -0.052225, -0.038241, -0.025465, -0.014212, -0.004714, 0.003428, 0.011472, 0.02206, 0.036956, 0.051255, 0.059724, 0.070578, 0.086496, 0.10727, 0.12321, 0.13844, 0.18006, 0.28968, 0.39413, 0.49858],
        [0.068693, 0.060554, 0.052286, 0.043766, 0.034845, 0.025372, 0.01508, 0.0035724, -0.0092297, -0.022705, -0.034504, -0.041549, -0.046081, -0.052219, -0.06095, -0.066454, -0.061458, -0.058267, -0.074244, -0.11613, -0.17193, -0.22927, -0.2866],
        [0.0099532, 0.010635, 0.0099363, 0.0071129, 0.0032754, 0.00020079, -0.0021446, -0.0035319, -0.0045294, -0.0060204, -0.0081974, -0.0092615, -0.0045697, 0.0049045, 0.013028, 0.018509, 0.015922, 0.0039426, 0.0029948, 0.010564, 0.0044049, -0.017749, -0.039904],
    ],
    // +50
    [
        [-0.040632, -0.014046, 0.0074221, 0.020361, 0.024629, 0.024894, 0.024423, 0.023588, 0.02668, 0.041905, 0.054203, 0.066294, 0.081469, 0.097117, 0.11898, 0.13924, 0.15672, 0.17937, 0.18968, 0.16627, 0.046013, -0.24651, -0.53904],
        [-0.19377, -0.16249, -0.13188, -0.10237, -0.074098, -0.046585, -0.018844, 0.010962, 0.048304, 0.094017, 0.12951, 0.16941, 0.20466, 0.24427, 0.28779, 0.34735, 0.42485, 0.46187, 0.48321, 0.56608, 0.76624, 0.91939, 1.0725],
        [0.14431, 0.11912, 0.093826, 0.068348, 0.042624, 0.016734, -0.0090047, -0.03374, -0.054135, -0.068633, -0.085998, -0.095067, -0.10321, -0.11543, -0.13996, -0.15327, -0.13722, -0.13843, -0.17952, -0.24398, -0.29461, -0.36625, -0.43789],
        [-0.033197, -0.031276, -0.030808, -0.03275, -0.036801, -0.040642, -0.043288, -0.045876, -0.0475, -0.0404, -0.026737, -0.016716, -0.0018888, 0.019545, 0.049953, 0.073907, 0.065694, 0.022846, 0.017533, 0.033678, -0.027625, -0.14096, -0.2543],
    ],
    // +100
    [
        [0.2394, 0.26209, 0.27504, 0.27271, 0.25986, 0.25248, 0.26147, 0.27425, 0.29151, 0.31438, 0.3381, 0.36976, 0.40458, 0.45437, 0.51417, 0.53017, 0.5714, 0.61759, 0.60559, 0.42594, 0.01709, -0.45389, -0.92486],
        [-0.94559, -0.76465, -0.58461, -0.40541, -0.22462, -0.037842, 0.1597, 0.37426, 0.61278, 0.85872, 1.084, 1.2819, 1.4371, 1.5866, 1.7616, 1.9651, 2.0808, 1.9148, 1.5795, 1.289, 0.98198, 0.65511, 0.32824],
        [0.31687, 0.2845, 0.25215, 0.22022, 0.1901, 0.16417, 0.14569, 0.13983, 0.1534, 0.18279, 0.22592, 0.29371, 0.35171, 0.33619, 0.19413, -0.010496, -0.23759, -0.47743, -0.58563, -0.37861, 0.029013, 0.4277, 0.82639],
        [-0.54943, -0.52617, -0.50576, -0.48968, -0.4748, -0.45124, -0.41014, -0.35935, -0.3041, -0.2422, -0.17325, -0.091742, 0.021435, 0.18803, 0.38608, 0.48325, 0.36522, 0.1395, 0.05089, -0.018982, -0.019105, -0.035324, -0.051543],
    ],
];

static WHITES_AT: [f32; 5] = [-100.0, -75.0, -50.0, -25.0, 0.0];
// Measured levels; some happen to lie near π/6 or ln 2.
#[allow(clippy::approx_constant)]
static WHITES_DOWN: [[f32; STEPS]; 5] = [
    // -100
    [
        0.0000, 0.0157, 0.0315, 0.0468, 0.0622, 0.0773, 0.0929, 0.1082, 0.1234, 0.1387, 0.1537, 0.1689, 0.1838, 0.1988, 0.2137, 0.2286, 0.2434, 0.2581, 0.2727,
        0.2872, 0.3016, 0.3160, 0.3304, 0.3444, 0.3586, 0.3725, 0.3864, 0.4001, 0.4137, 0.4273, 0.4405, 0.4538, 0.4668, 0.4800, 0.4932, 0.5067, 0.5202, 0.5335,
        0.5469, 0.5601, 0.5733, 0.5862, 0.5993, 0.6120, 0.6249, 0.6375, 0.6501, 0.6626, 0.6750, 0.6874, 0.6996, 0.7120, 0.7241, 0.7366, 0.7489, 0.7617, 0.7748,
        0.7886, 0.8037, 0.8205, 0.8410, 0.8660, 0.8993, 0.9440, 1.0000,
    ],
    // -75
    [
        0.0000, 0.0157, 0.0315, 0.0468, 0.0622, 0.0773, 0.0929, 0.1082, 0.1234, 0.1387, 0.1537, 0.1689, 0.1838, 0.1988, 0.2137, 0.2286, 0.2435, 0.2582, 0.2729,
        0.2873, 0.3019, 0.3164, 0.3307, 0.3448, 0.3590, 0.3731, 0.3870, 0.4007, 0.4146, 0.4282, 0.4417, 0.4552, 0.4685, 0.4818, 0.4953, 0.5090, 0.5228, 0.5365,
        0.5501, 0.5638, 0.5774, 0.5908, 0.6045, 0.6179, 0.6313, 0.6447, 0.6580, 0.6716, 0.6849, 0.6985, 0.7122, 0.7260, 0.7397, 0.7540, 0.7684, 0.7833, 0.7989,
        0.8149, 0.8324, 0.8513, 0.8726, 0.8967, 0.9255, 0.9600, 1.0000,
    ],
    // -50
    [
        0.0000, 0.0157, 0.0315, 0.0468, 0.0622, 0.0773, 0.0930, 0.1082, 0.1235, 0.1388, 0.1538, 0.1691, 0.1840, 0.1990, 0.2141, 0.2291, 0.2439, 0.2588, 0.2735,
        0.2882, 0.3031, 0.3174, 0.3320, 0.3464, 0.3609, 0.3752, 0.3893, 0.4035, 0.4176, 0.4316, 0.4455, 0.4595, 0.4732, 0.4870, 0.5010, 0.5154, 0.5297, 0.5439,
        0.5582, 0.5725, 0.5869, 0.6010, 0.6154, 0.6297, 0.6440, 0.6585, 0.6728, 0.6874, 0.7019, 0.7168, 0.7316, 0.7467, 0.7618, 0.7774, 0.7931, 0.8092, 0.8257,
        0.8428, 0.8607, 0.8794, 0.8996, 0.9211, 0.9449, 0.9711, 1.0000,
    ],
    // -25
    [
        0.0000, 0.0157, 0.0315, 0.0468, 0.0623, 0.0775, 0.0932, 0.1085, 0.1239, 0.1394, 0.1544, 0.1699, 0.1849, 0.2003, 0.2154, 0.2306, 0.2456, 0.2609, 0.2759,
        0.2908, 0.3059, 0.3209, 0.3359, 0.3507, 0.3656, 0.3804, 0.3952, 0.4099, 0.4247, 0.4395, 0.4540, 0.4687, 0.4832, 0.4980, 0.5128, 0.5279, 0.5429, 0.5579,
        0.5729, 0.5879, 0.6030, 0.6180, 0.6331, 0.6482, 0.6634, 0.6787, 0.6939, 0.7094, 0.7246, 0.7402, 0.7557, 0.7715, 0.7871, 0.8033, 0.8192, 0.8355, 0.8519,
        0.8686, 0.8857, 0.9030, 0.9211, 0.9395, 0.9589, 0.9788, 1.0000,
    ],
    // 0
    [
        0.000000, 0.015625, 0.031250, 0.046875, 0.062500, 0.078125, 0.093750, 0.109375, 0.125000, 0.140625, 0.156250, 0.171875, 0.187500, 0.203125, 0.218750,
        0.234375, 0.250000, 0.265625, 0.281250, 0.296875, 0.312500, 0.328125, 0.343750, 0.359375, 0.375000, 0.390625, 0.406250, 0.421875, 0.437500, 0.453125,
        0.468750, 0.484375, 0.500000, 0.515625, 0.531250, 0.546875, 0.562500, 0.578125, 0.593750, 0.609375, 0.625000, 0.640625, 0.656250, 0.671875, 0.687500,
        0.703125, 0.718750, 0.734375, 0.750000, 0.765625, 0.781250, 0.796875, 0.812500, 0.828125, 0.843750, 0.859375, 0.875000, 0.890625, 0.906250, 0.921875,
        0.937500, 0.953125, 0.968750, 0.984375, 1.000000,
    ],
];
static BLACKS_AT: [f32; 9] = [-100.0, -75.0, -50.0, -25.0, 0.0, 25.0, 50.0, 75.0, 100.0];
// Measured levels; some happen to lie near π/6 or ln 2.
#[allow(clippy::approx_constant)]
static BLACKS: [[f32; STEPS]; 9] = [
    // -100
    [
        0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0054, 0.0206, 0.0407, 0.0602,
        0.0799, 0.1006, 0.1221, 0.1443, 0.1665, 0.1887, 0.2106, 0.2321, 0.2533, 0.2740, 0.2945, 0.3144, 0.3344, 0.3539, 0.3735, 0.3933, 0.4135, 0.4341, 0.4551,
        0.4767, 0.4988, 0.5214, 0.5436, 0.5658, 0.5877, 0.6094, 0.6310, 0.6524, 0.6735, 0.6944, 0.7153, 0.7358, 0.7563, 0.7763, 0.7963, 0.8159, 0.8354, 0.8548,
        0.8735, 0.8925, 0.9109, 0.9293, 0.9475, 0.9654, 0.9827, 1.0000,
    ],
    // -75
    [
        0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0012, 0.0150, 0.0338, 0.0521, 0.0692, 0.0867, 0.1049, 0.1236, 0.1427, 0.1622, 0.1816,
        0.2010, 0.2203, 0.2391, 0.2579, 0.2762, 0.2945, 0.3125, 0.3304, 0.3482, 0.3659, 0.3835, 0.4007, 0.4181, 0.4354, 0.4527, 0.4704, 0.4886, 0.5072, 0.5258,
        0.5445, 0.5630, 0.5815, 0.5997, 0.6181, 0.6363, 0.6545, 0.6725, 0.6905, 0.7085, 0.7264, 0.7442, 0.7619, 0.7795, 0.7970, 0.8145, 0.8317, 0.8491, 0.8663,
        0.8831, 0.9003, 0.9171, 0.9340, 0.9506, 0.9673, 0.9836, 1.0000,
    ],
    // -50
    [
        0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0141, 0.0339, 0.0519, 0.0681, 0.0846, 0.1010, 0.1179, 0.1347, 0.1519, 0.1692, 0.1868, 0.2042, 0.2217, 0.2390,
        0.2560, 0.2731, 0.2900, 0.3069, 0.3236, 0.3404, 0.3569, 0.3735, 0.3899, 0.4065, 0.4230, 0.4392, 0.4557, 0.4720, 0.4884, 0.5051, 0.5222, 0.5391, 0.5560,
        0.5729, 0.5898, 0.6066, 0.6232, 0.6402, 0.6568, 0.6735, 0.6902, 0.7067, 0.7234, 0.7398, 0.7565, 0.7730, 0.7895, 0.8058, 0.8223, 0.8386, 0.8550, 0.8712,
        0.8874, 0.9037, 0.9197, 0.9360, 0.9520, 0.9682, 0.9840, 1.0000,
    ],
    // -25
    [
        0.0000, 0.0000, 0.0022, 0.0177, 0.0369, 0.0540, 0.0705, 0.0863, 0.1023, 0.1184, 0.1344, 0.1509, 0.1670, 0.1832, 0.1997, 0.2161, 0.2324, 0.2487, 0.2650,
        0.2811, 0.2973, 0.3134, 0.3296, 0.3455, 0.3616, 0.3776, 0.3936, 0.4095, 0.4255, 0.4415, 0.4574, 0.4733, 0.4892, 0.5052, 0.5213, 0.5374, 0.5536, 0.5697,
        0.5858, 0.6019, 0.6181, 0.6340, 0.6501, 0.6661, 0.6822, 0.6983, 0.7142, 0.7302, 0.7462, 0.7622, 0.7781, 0.7942, 0.8099, 0.8259, 0.8418, 0.8577, 0.8737,
        0.8893, 0.9053, 0.9210, 0.9369, 0.9527, 0.9685, 0.9842, 1.0000,
    ],
    // 0
    [
        0.000000, 0.015625, 0.031250, 0.046875, 0.062500, 0.078125, 0.093750, 0.109375, 0.125000, 0.140625, 0.156250, 0.171875, 0.187500, 0.203125, 0.218750,
        0.234375, 0.250000, 0.265625, 0.281250, 0.296875, 0.312500, 0.328125, 0.343750, 0.359375, 0.375000, 0.390625, 0.406250, 0.421875, 0.437500, 0.453125,
        0.468750, 0.484375, 0.500000, 0.515625, 0.531250, 0.546875, 0.562500, 0.578125, 0.593750, 0.609375, 0.625000, 0.640625, 0.656250, 0.671875, 0.687500,
        0.703125, 0.718750, 0.734375, 0.750000, 0.765625, 0.781250, 0.796875, 0.812500, 0.828125, 0.843750, 0.859375, 0.875000, 0.890625, 0.906250, 0.921875,
        0.937500, 0.953125, 0.968750, 0.984375, 1.000000,
    ],
    // +25
    [
        0.0000, 0.0184, 0.0363, 0.0531, 0.0703, 0.0875, 0.1052, 0.1225, 0.1395, 0.1563, 0.1729, 0.1897, 0.2058, 0.2221, 0.2381, 0.2541, 0.2700, 0.2859, 0.3018,
        0.3175, 0.3333, 0.3490, 0.3647, 0.3802, 0.3960, 0.4115, 0.4270, 0.4426, 0.4581, 0.4736, 0.4890, 0.5044, 0.5197, 0.5346, 0.5495, 0.5643, 0.5792, 0.5940,
        0.6088, 0.6237, 0.6386, 0.6533, 0.6682, 0.6830, 0.6980, 0.7127, 0.7275, 0.7425, 0.7573, 0.7723, 0.7871, 0.8022, 0.8170, 0.8321, 0.8471, 0.8622, 0.8775,
        0.8923, 0.9077, 0.9228, 0.9384, 0.9535, 0.9690, 0.9844, 1.0000,
    ],
    // +50
    [
        0.0000, 0.0215, 0.0417, 0.0599, 0.0793, 0.0986, 0.1180, 0.1369, 0.1553, 0.1734, 0.1910, 0.2084, 0.2251, 0.2418, 0.2581, 0.2744, 0.2905, 0.3066, 0.3226,
        0.3383, 0.3544, 0.3699, 0.3857, 0.4012, 0.4168, 0.4323, 0.4476, 0.4629, 0.4782, 0.4936, 0.5087, 0.5236, 0.5380, 0.5523, 0.5664, 0.5805, 0.5946, 0.6086,
        0.6228, 0.6368, 0.6509, 0.6649, 0.6792, 0.6931, 0.7073, 0.7214, 0.7355, 0.7497, 0.7638, 0.7783, 0.7924, 0.8068, 0.8211, 0.8357, 0.8501, 0.8648, 0.8794,
        0.8941, 0.9090, 0.9237, 0.9390, 0.9539, 0.9692, 0.9845, 1.0000,
    ],
    // +75
    [
        0.0000, 0.0252, 0.0476, 0.0678, 0.0894, 0.1109, 0.1324, 0.1528, 0.1726, 0.1916, 0.2100, 0.2279, 0.2452, 0.2621, 0.2786, 0.2952, 0.3113, 0.3276, 0.3436,
        0.3594, 0.3752, 0.3909, 0.4065, 0.4219, 0.4372, 0.4525, 0.4676, 0.4827, 0.4978, 0.5128, 0.5272, 0.5415, 0.5551, 0.5686, 0.5819, 0.5953, 0.6087, 0.6220,
        0.6354, 0.6488, 0.6622, 0.6754, 0.6889, 0.7023, 0.7157, 0.7292, 0.7426, 0.7563, 0.7697, 0.7834, 0.7971, 0.8110, 0.8247, 0.8388, 0.8529, 0.8669, 0.8814,
        0.8955, 0.9101, 0.9246, 0.9395, 0.9542, 0.9694, 0.9845, 1.0000,
    ],
    // +100
    [
        0.0000, 0.0295, 0.0540, 0.0767, 0.1009, 0.1247, 0.1481, 0.1701, 0.1911, 0.2110, 0.2299, 0.2483, 0.2657, 0.2829, 0.2996, 0.3162, 0.3325, 0.3487, 0.3647,
        0.3804, 0.3960, 0.4114, 0.4269, 0.4420, 0.4571, 0.4721, 0.4870, 0.5018, 0.5164, 0.5306, 0.5443, 0.5579, 0.5708, 0.5835, 0.5962, 0.6089, 0.6216, 0.6342,
        0.6469, 0.6595, 0.6722, 0.6848, 0.6977, 0.7104, 0.7232, 0.7361, 0.7489, 0.7620, 0.7749, 0.7880, 0.8012, 0.8146, 0.8279, 0.8415, 0.8551, 0.8689, 0.8828,
        0.8968, 0.9111, 0.9254, 0.9400, 0.9545, 0.9695, 0.9845, 1.0000,
    ],
];

/// How a curve row acts on a pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowOn {
    /// Each channel, hue kept (in linear ProPhoto): the clipping ways.
    Channels,
    /// The luminance, the pixel scaled by its change: the compressing ways.
    Luminance,
}

/// The Whites curve at slider value `whites` (−100 … 100) as a row over grey levels 0, 1/64, …,
/// 1 (sRGB-encoded), with how it acts. Whites up needs the image's `points`; without them it is
/// `None`, as it is at 0.
pub(crate) fn whites_row(whites: f32, points: Option<Tones>) -> Option<([f32; STEPS], RowOn)> {
    if !whites.is_finite() || whites == 0.0 {
        return None;
    }
    if whites < 0.0 {
        return Some((interpolate(&WHITES_AT, &WHITES_DOWN, whites.max(-100.0)), RowOn::Luminance));
    }
    let [_, med, wp, _] = points?;
    if !(med.is_finite() && wp.is_finite()) {
        return None;
    }
    let (g, m) = ((-wp).max(0.0), (wp - med).max(0.0));
    // The move at one fitted slider value, at log2 luminance `u`.
    let fitted = |t: &[[f32; WHITES_KNOTS]; 4], u: f32| {
        let x = ((u - wp + 10.0) / 0.5).clamp(0.0, (WHITES_KNOTS - 1) as f32);
        let i = (x as usize).min(WHITES_KNOTS - 2);
        let s = x - i as f32;
        let at = |r: usize| t[r][i] + (t[r][i + 1] - t[r][i]) * s;
        at(0) + at(1) * g + at(2) * g * g + at(3) * m / 10.0
    };
    // Between fitted values the move interpolates (from none at 0).
    let a = whites.min(100.0);
    let k = WHITES_UP_AT.iter().position(|v| a <= *v).unwrap_or(WHITES_UP_AT.len() - 1);
    let (a0, f) = if k == 0 { (0.0, a / WHITES_UP_AT[0]) } else { (WHITES_UP_AT[k - 1], (a - WHITES_UP_AT[k - 1]) / (WHITES_UP_AT[k] - WHITES_UP_AT[k - 1])) };
    let shift = |u: f32| {
        let hi = fitted(&WHITES_UP[k], u);
        let lo = if k == 0 || a0 == 0.0 { 0.0 } else { fitted(&WHITES_UP[k - 1], u) };
        // Whites up never darkens, and stays within what was measured.
        (lo + (hi - lo) * f).clamp(0.0, 4.0)
    };
    let mut row: [f32; STEPS] = std::array::from_fn(|k| {
        let v = super::srgb_to_linear(k as f32 / (STEPS - 1) as f32);
        if v <= 0.0 {
            return 0.0;
        }
        super::linear_to_srgb((v * shift(v.log2()).exp2()).min(1.0))
    });
    for k in 1..STEPS {
        row[k] = row[k].max(row[k - 1]);
    }
    Some((row, RowOn::Channels))
}

/// The Blacks curve at slider value `blacks` (−100 … 100) as a row, with how it acts; `None` at 0.
pub(crate) fn blacks_row(blacks: f32) -> Option<([f32; STEPS], RowOn)> {
    if !blacks.is_finite() || blacks == 0.0 {
        return None;
    }
    let on = if blacks < 0.0 { RowOn::Channels } else { RowOn::Luminance };
    Some((interpolate(&BLACKS_AT, &BLACKS, blacks.clamp(-100.0, 100.0)), on))
}

/// One row on one straight pixel encoded with `enc`; `m` from [`primaries`].
pub(crate) fn row_pixel(q: [f32; 4], row: &[f32; STEPS], on: RowOn, enc: Encoding, m: Option<&super::exposure::Primaries>) -> [f32; 4] {
    match on {
        RowOn::Channels => pixel(q, row, enc, m),
        RowOn::Luminance => {
            let lin = [0, 1, 2].map(|c| enc.decode(q[c]));
            let y = super::luma_linear(lin).max(0.0);
            if y <= 1e-7 {
                return q;
            }
            let k = super::exposure::curve(row, y) / y;
            let mut o = q;
            for c in 0..3 {
                o[c] = enc.encode(lin[c] * k);
            }
            o
        }
    }
}

/// Applies Whites then Blacks (rows from [`whites_row`] and [`blacks_row`]) to straight RGBA
/// pixels (`width` per row) encoded with `enc`.
///
/// `fix` is Whites down's measured colour correction ([`colour_lut`](super::colour_lut)), added
/// right after the Whites curve.
pub(crate) fn apply(
    px: &mut [[f32; 4]],
    width: usize,
    whites: Option<&([f32; STEPS], RowOn)>,
    blacks: Option<&([f32; STEPS], RowOn)>,
    fix: Option<&super::colour_lut::Correction>,
    enc: Encoding,
) {
    if whites.is_none() && blacks.is_none() {
        return;
    }
    let m = primaries(enc);
    par_rows(px, width, 1, |_, row| {
        for q in row.iter_mut() {
            if let Some((r, on)) = whites {
                let input = [q[0], q[1], q[2]];
                *q = row_pixel(*q, r, *on, enc, m.as_ref());
                if let Some(f) = fix {
                    let o = f.apply(input, [q[0], q[1], q[2]], enc);
                    q[..3].copy_from_slice(&o);
                }
            }
            if let Some((r, on)) = blacks {
                *q = row_pixel(*q, r, *on, enc, m.as_ref());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey(row: &[f32; STEPS], level: f32) -> f32 {
        let lin = super::super::exposure::curve(row, super::super::srgb_to_linear(level / 255.0));
        super::super::linear_to_srgb(lin) * 255.0
    }

    #[test]
    fn fixed_rows_match_photoshop() {
        // Photoshop 2026 on the 256-level ramp (sRGB levels in and out).
        let (w, _) = whites_row(-100.0, None).unwrap();
        assert!((grey(&w, 224.0) - 198.3).abs() < 1.0 && (grey(&w, 160.0) - 146.7).abs() < 1.0);
        let (b, on) = blacks_row(-100.0).unwrap();
        assert_eq!(on, RowOn::Channels);
        assert!(grey(&b, 48.0) < 0.5 && (grey(&b, 96.0) - 48.7).abs() < 1.0);
        let (b, on) = blacks_row(100.0).unwrap();
        assert_eq!(on, RowOn::Luminance);
        assert!((grey(&b, 24.0) - 37.9).abs() < 1.0 && (grey(&b, 96.0) - 116.9).abs() < 1.0);
        // Between measured values the rows interpolate.
        let (b, _) = blacks_row(-62.5).unwrap();
        let v = grey(&b, 48.0);
        assert!(v > 17.8 && v < 34.5, "{v}");
    }

    #[test]
    fn whites_up_adapts_to_the_white_point() {
        // A white point below white gets far more lift (Photoshop +100 at a level 2.5 stops below
        // the white point: about 0.6 stops on a render that reaches white, 1.9 on a photograph
        // whose brightest levels stop 0.55 stops short of it).
        let bright = whites_row(100.0, Some([-5.25, -1.36, 0.0, -1.7])).unwrap().0;
        let dull = whites_row(100.0, Some([-7.3, -4.3, -0.55, -4.0])).unwrap().0;
        let stops = |r: &[f32; STEPS], u: f32| {
            let level = super::super::linear_to_srgb(u.exp2()) * 255.0;
            (super::super::srgb_to_linear(grey(r, level) / 255.0) / u.exp2()).log2()
        };
        let (b, d) = (stops(&bright, -2.5), stops(&dull, -3.05));
        assert!(b > 0.3 && b < 1.0 && d > 1.4 && d < 2.4, "{b} {d}");
        // +25 lifts less than half as much as +50, which lifts less than half of +100.
        let at = |v: f32| stops(&whites_row(v, Some([-7.3, -4.3, -0.55, -4.0])).unwrap().0, -3.05);
        assert!(at(25.0) < 0.5 * at(50.0) && at(50.0) < 0.5 * at(100.0), "{} {} {}", at(25.0), at(50.0), at(100.0));
        // Without the image's points there is no Whites up; bad points are ignored.
        assert!(whites_row(50.0, None).is_none());
        assert!(whites_row(50.0, Some([-6.0, -2.0, f32::NAN, -2.0])).is_none());
        assert!(whites_row(50.0, Some([-6.0, f32::INFINITY, 0.0, -2.0])).is_none());
        assert!(whites_row(0.0, Some([-5.0, -2.0, 0.0, -2.0])).is_none());
        // White stays white and the row never decreases.
        assert!(bright.windows(2).all(|p| p[1] >= p[0]) && bright[STEPS - 1] <= 1.0);
    }
}
