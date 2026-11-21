<p align="center">
  <a href="https://getartcraft.com/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/brand/artcraft-logo-white.svg">
      <img alt="ArtCraft" src="docs/brand/artcraft-logo.svg" width="200">
    </picture>
  </a>
</p>

<h1 align="center">PhotoCraft</h1>

<p align="center">
  <b>معالجة الصور؛ إعادة تنفيذ مفتوحة المصدر لبرنامج Adobe Photoshop وفق منهجية الغرفة النظيفة، مبنية بلغة Rust خالصة.</b><br>
  طبقات، وأقنعة، وطبقات ضبط، وأنماط طبقات، ونصوص، ومتجهات، وفرش، وملفات PSD حقيقية،<br>
  في تطبيق أصلي مكتوب بلغة Rust من أوله إلى آخره. مفتوح المصدر، يعمل دون اتصال، ولك وحدك.
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="macOS · Windows · Linux · FreeBSD · الويب" src="https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20FreeBSD%20%C2%B7%20Web-native-2f7bf5?style=flat-square">
  <img alt="الترخيص: MIT أو Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
  <img alt="الحالة: نسخة ألفا مبكرة" src="https://img.shields.io/badge/status-early%20alpha-d69e2e?style=flat-square">
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="انضم إلى مجتمع ArtCraft على Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<p align="center">
  <a href="https://getartcraft.com/apps/photocraft"><b>PhotoCraft على getartcraft.com</b></a> ·
  <a href="https://getartcraft.com/">ArtCraft</a> ·
  <a href="https://getartcraft.com/apps">كل Crafting Apps</a>
</p>

<p align="center">
  <a href="README.md">English</a> ·
  <a href="README.ar.md">العربية</a>
</p>

<div dir="rtl">

<br>

<p align="center">
  <img src="docs/images/photocraft-demo.jpg" alt="PhotoCraft يحرّر «الموجة الكبرى» لهوكوساي: بطاقة تعريف بظل مسقط، وطبقتا نص Title وCredit، وطبقتا ضبط Vibrance وCurves، ومحرر المنحنيات مرسوم فوق المدرج التكراري للصورة" width="100%">
  <br>
  <sub>بطاقة تعريف بظل مسقط، ونص حيّ، وطبقتا ضبط Vibrance وCurves، مع فتح محرر المنحنيات.<br>
  <i>الموجة الكبرى قبالة كاناغاوا</i>، كاتسوشيكا هوكوساي، نحو 1831</sub>
</p>

> [!NOTE]
> **ArtCraft مجتمع من الفنانين من كل المشارب.** فن رقمي، وفن توليدي، وموسيقى، وألعاب &mdash; إن كنت تصنع الأشياء فأنت منّا. **[تعالَ وسلّم علينا في Discord](https://discord.gg/artcraft).**

<p align="center">
  <a href="#المزايا">المزايا</a> ·
  <a href="#كل-ما-في-الصندوق">كل ما في الصندوق</a> ·
  <a href="#psd-بلا-مساومة">PSD</a> ·
  <a href="#مبني-للوكلاء">الوكلاء</a> ·
  <a href="#تحت-الغطاء">تحت الغطاء</a> ·
  <a href="#البدء-السريع">البدء السريع</a> ·
  <a href="#التنزيلات">التنزيلات</a> ·
  <a href="#crafting-apps">Crafting Apps</a> ·
  <a href="https://discord.gg/artcraft">Discord</a>
</p>

<br>

<table>
  <tr>
    <td width="25%" valign="top">
      <h3>🎛️ مألوف بالتصميم</h3>
      القوائم والاختصارات واللوحات والأدوات حيث تتوقعها يداك، من ⌘J إلى ⇧⌘D. إن كنت تعرف Photoshop فأنت تعرف PhotoCraft بالفعل.
    </td>
    <td width="25%" valign="top">
      <h3>⚡ أصلي وسريع</h3>
      مُركِّب على وحدة معالجة الرسوميات عبر wgpu ‏(Metal وVulkan وDX12 وWebGPU)، وبلاطات بالنسخ عند الكتابة، ومرشحات متعددة الخيوط. لا Electron، ولا عرض ويب، ولا انتظار.
    </td>
    <td width="25%" valign="top">
      <h3>🗂️ ملفات PSD حقيقية</h3>
      افتح مستندات Photoshop الطبقية وحرّرها واحفظها. تحافظ إعادة الحفظ على تصيير 307 من ملفات اختبار psd-tools البالغ عددها 309.
    </td>
    <td width="25%" valign="top">
      <h3>🤖 جاهز للوكلاء</h3>
      كل إجراء أمرٌ قابل للتنفيذ، فيمكنك قيادة المحرك نفسه من الواجهة أو سطر الأوامر أو قناة تحكم بصيغة JSON أو خادم MCP.
    </td>
  </tr>
</table>

<br>

## المزايا

كل لقطة شاشة هنا هي التطبيق الحقيقي وهو يعمل على أعمال فنية من الملك العام، مصيَّرة خارج الشاشة عبر قناة التحكم الخاصة به.

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-adjustments.jpg" alt="«انطباع، شروق الشمس» لمونيه مع طبقتَي ضبط Levels وVibrance؛ ومحرر Levels ولوحة المدرج التكراري مع المتوسط والانحراف المعياري والوسيط مفتوحان على اليمين" width="100%">
      <br>
      <sub>طبقتا ضبط Levels وVibrance، مع لوحة المدرج التكراري الحيّة.<br><i>انطباع، شروق الشمس</i>، كلود مونيه، 1872</sub>
      <h3>حرّر بلا ندم</h3>
      طبقات الضبط تُبقي كل تعديل حياً. راكِب Levels وCurves وVibrance وHue/Saturation وغيرها الكثير، وقنّعها على منطقة بعينها، وأعد ترتيبها، أو عطّلها، وتبقى بكسلاتك الأصلية دون أي مساس.
      <br><br>
      <b>طبقات ضبط يبلغ عددها 16</b> يمكن تطبيقها أيضاً مباشرة على البكسلات، تشمل Curves بتحرير كل قناة على حدة، وLevels بمدرج تكراري حيّ، وBlack &amp; White، وChannel Mixer، وGradient Map، وPhoto Filter، وSelective Color، وColor Lookup ‏(.cube و.3dl و.look). إضافة إلى Shadows/Highlights وReplace Color وMatch Color وHDR Toning وDesaturate وEqualize.
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-layer-styles.jpg" alt="مربع حوار Layer Style يحرّر توهجاً خارجياً على طبقة النص EARTHRISE مع تمكين الحد أيضاً، فوق صورة «شروق الأرض» من مهمة أبولو 8" width="100%">
      <br>
      <sub>توهج خارجي وحد على طبقة نص حيّة، في مربع حوار Layer Style.<br><i>شروق الأرض</i>، ويليام أندرز / ناسا، 1968</sub>
      <h3>أنماط تجعل اللقطة تبيع نفسها</h3>
      إسقاط الظل، والظل الداخلي، والتوهج الخارجي والداخلي، والشطف والنقش، والساتان، والحد، وتراكب اللون والتدرج والنقشة — كلها حيّة على أي طبقة، والنصوص شاملة. تأتي النقشات من مكتبة (نقشات مدمجة، أو عبر تحرير › تعريف نقشة، أو استيراد/تصدير <code>.pat</code>) ومن كتل <code>Patt</code> في ملفات PSD.
      <br><br>
      انسخ الأنماط والصقها بين الطبقات، وأخفِ كل التأثيرات دفعة واحدة، وافتح الأنماط مباشرة من ملفات PSD الخاصة بك، مصيَّرة لتطابق ما يعرضه Photoshop.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-masks.jpg" alt="«الفتاة ذات القرط اللؤلؤي» لفيرمير: تحديد بيضاوي حول الوجه، وطبقة Hue/Saturation بقناع بيضاوي يجعل كل ما عدا الوجه رمادياً" width="100%">
      <br>
      <sub>تحديد بيضاوي يصير قناع طبقة Hue/Saturation، فلا يحتفظ بلونه سوى الوجه.<br><i>الفتاة ذات القرط اللؤلؤي</i>، يوهانس فيرمير، نحو 1665</sub>
      <h3>تحديدات تفهم صورتك</h3>
      أدوات التحديد والحبال والعصا السحرية حين تحتاج الدقة؛ والتحديد السريع وتحديد الكائنات وتحديد الموضوع حين تريد أن يتولى الحاسوب التتبع نيابة عنك؛ وتحديد وقناع لتنقيح الحواف التي ترقّ حتى حدّ الشعرة.
      <br><br>
      تنعيم الحواف، وتوسيع، وتقليص، وتمهيد، وتوسيع نحو المشابه، وإعادة التحديد. حوّل أي تحديد إلى قناع طبقة أو مسار متجه أو شكل. التحديد الذكي يعمل على جهازك، بلا سحابة وبلا حساب.
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-type.jpg" alt="«بين جبال سييرا نيفادا» لبيرشتات مع ترويسة SIERRA NEVADA تُحرَّر على اللوحة بخط Georgia، وسطر تقديم، وتعليق من فقرة؛ وعناصر تحكم الحرف والفقرة مفتوحة" width="100%">
      <br>
      <sub>ترويسة تُحرَّر في موضعها، مع سطر تقديم وفقرة من نص المتن.<br><i>بين جبال سييرا نيفادا، كاليفورنيا</i>، ألبرت بيرشتات، 1868</sub>
      <h3>نص يُنضَد بجمال</h3>
      نص نقطي وفقري، يُحرَّر مباشرة على اللوحة، مع تحكم كامل من لوحتي الحرف والفقرة: الخط والوزن والحجم والإراحة (leading) والتباعد الحرفي (tracking) والمحاذاة واللون. كما تدعم <a href="docs/color-picker.md">لوحة ألوان النص الكاملة</a> أخذ العينات من الشاشة بعدسة بكسلات مكبّرة.
      <br><br>
      تبقى طبقات النص قابلة للتحرير، وتقبل أنماط الطبقات، وتكتمل دورتها ذهاباً وإياباً عبر ملفات PSD.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-vector.jpg" alt="شارة لوتس مبنية من طبقات أشكال (اللوتس والماء والشمس والشارة والحلقة المنقطة) فوق «زنابق الماء» لمونيه، مع تحديد نقط ارتكاز مسار اللوتس" width="100%">
      <br>
      <sub>شارة من طبقات الأشكال: لوتسة معبأة بتدرج، ونجمة، وحلقة منقطة.<br><i>زنابق الماء</i>، كلود مونيه، 1906</sub>
      <h3>متجهات بدقة البكسل</h3>
      أدوات المستطيل، والقطع الناقص، والمثلث، والمضلع، والخط، والقلم، والقلم الحر، وقلم الانحناء، مع طبقات أشكال مستقلة عن الدقة، وتعبئات متدرجة، وحدود متقطعة مصفوفة بعناية.
      <br><br>
      اجمع الأشكال (توحيداً، وطرحاً، وتقاطعاً، واستبعاداً)، واحتفظ بالمسارات في لوحة المسارات، واستخدمها أقنعة متجهة، أو طبّق عليها الحد والتعبئة. 116 من أصل 116 طبقة شكل في مجموعة PSD لدينا تطابق بكسلات Photoshop.
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-filters.jpg" alt="مربع حوار مرشح الالتواء بزاوية 320 درجة، يعاين حياً داخل تحديد بيضاوي على «الليلة النجومية» لفان جوخ" width="100%">
      <br>
      <sub>مرشح الالتواء يعاين حياً على اللوحة، داخل التحديد فقط.<br><i>الليلة النجومية</i>، فينسنت فان جوخ، 1889</sub>
      <h3>عاين قبل أن تُطبّق</h3>
      كل مربع حوار مرشح يعاين حياً على اللوحة، من خلال تحديدك. ضبابيات (غاوسي، وصندوقي، وحركي، وشعاعي، ووسطي، وذكي، وعدسي، وشكلي، ومعرض الضبابية: إمالة وإزاحة، وقزحية، وحقل، ودوران، ومسار)، وزيادة الحدة، وتقليل التشويش، وتحريفات (التواء، وموجة، وتموج، وإزاحة، وقصّ، وزجزاج…)، وتنقيط، وتنميط (رسم زيتي، وريح، وبثق…)، وتصيير (غيوم، وألياف، وتوهج عدسي، وتأثيرات إضاءة) وغيرها.
      <br><br>
      طبّق المرشحات على كائن ذكي وتبقى قابلة للتحرير: غيّرها أو أخفها أو أعد ترتيبها أو قنّعها في أي وقت.
      <br><br>
      الضبابيات ذات أنصاف الأقطار الكبيرة تستخدم تمريرات صندوقية بجمع جارٍ عبر كل الأنوية: غاوسي بنصف قطر 180 على صورة 3.6 ميغابكسل يستغرق أقل من ثانية.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-transform.jpg" alt="مقابض التحويل الحر حول نسخة مدوّرة من «جبال تيتون ونهر سنيك» لأنسل آدمز، مع لوحة السجل تسرد خطوات الفتح وتكرار الطبقة والتحويل الحر وغيرها" width="100%">
      <br>
      <sub>تحويل حر على نسخة مدوّرة، وكل خطوة مدرجة في السجل.<br><i>جبال تيتون ونهر سنيك</i>، أنسل آدمز، 1942</sub>
      <h3>شكّلها كما تشاء</h3>
      تحويل حر بالتحجيم والتدوير والإمالة والتحريف والمنظور؛ تدوير وقلب مضبوطان بـ 90° و180°؛ وإعادة التحويل. الطبقات والنصوص والأشكال والأقنعة والتحديدات كلها تتحول معاً.
      <br><br>
      سجل كامل، وتبديل الحالة الأخيرة، وفرشاة السجل — كل خطوة قابلة للتراجع، ولو لمسة فرشاة واحدة في كل مرة.
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/photocraft-export-light.jpg" alt="مربع حوار «تصدير باسم» بالنسق الفاتح فوق «القبلة» لكليمت: JPG بجودة 90 وتحجيم 50 في المئة، مع معاينة وحجم مقدَّر يناهز 684K" width="100%">
      <br>
      <sub>«تصدير باسم» بالنسق الفاتح، مع معاينة وتقدير لحجم الملف.<br><i>القبلة</i>، غوستاف كليمت، 1907–1908</sub>
      <h3>انشرها أينما تريد</h3>
      «تصدير باسم» بضبط الصيغة والجودة والشفافية والتحجيم، إضافة إلى معاينة وتقدير فوري لحجم الملف. وتصدير سريع إلى PNG بنقرة واحدة.
      <br><br>
      اختر نسق Pro الداكن، أو أنساق Studio الفاتحة الرحبة، أو مظهراً كلاسيكياً.
    </td>
  </tr>
</table>

<br>

## كل ما في الصندوق

<table>
  <tr>
    <td width="33%" valign="top">
      <h4>🧰 67 أداة</h4>
      نقل · لوحة الرسم · التحديد المستطيل والبيضوي · تحديد صف واحد · تحديد عمود واحد · حبل · لاسو مضلع · لاسو مغناطيسي · تحديد الكائنات · التحديد السريع · العصا السحرية · اقتصاص · قصّ منظوري · شريحة · تحديد الشريحة · إطار · قطارة · أخذ عينات اللون · مسطرة · ملاحظة · عدّ · إزالة · الفرشاة الشافية الفورية · الفرشاة الشافية · رقعة · نقل واعٍ بالمحتوى · العين الحمراء · فرشاة · قلم رصاص · فرشاة مازجة · استبدال اللون · خاتم الاستنساخ · خاتم النقشة · فرشاة السجل · ممحاة · ممحاة الخلفية · الممحاة السحرية · تدرج · دلو التعبئة · ضبابية · زيادة الحدة · تلطيخ · تفتيح · تغميق · إسفنجة · قلم · قلم حر · قلم الانحناء · إضافة نقطة ارتكاز · حذف نقطة ارتكاز · تحويل النقاط · نص أفقي ورأسي · قناع نص أفقي ورأسي · تحديد المسار · التحديد المباشر · مستطيل · قطع ناقص · مثلث · مضلع · خط · شكل مخصص · يد · عرض التدوير · تكبير
    </td>
    <td width="33%" valign="top">
      <h4>🖌️ محرك فرشاة حقيقي</h4>
      ديناميكية الشكل، والتبعثر، والنسيج، والفرشاة المزدوجة، وديناميكية اللون، والنقل، ووضعية الفرشاة، والحواف الرطبة، والتراكم، والتنعيم (بما فيه وضع الخيط المسبول)، كلها تستجيب لضغط القلم وميله ودورانه واتجاهه. إعدادات فرشاة مسبقة، وتعريف فرشاة من التحديد، وضربات حتمية قابلة لإعادة التشغيل.
    </td>
    <td width="33%" valign="top">
      <h4>🗃️ طبقات على أصولها</h4>
      مجموعات، وأقنعة اقتطاع، وأقنعة بكسلية ومتجهة، وطبقات تعبئة (صلدة ومتدرجة ونقشة)، وطبقات ضبط، وكائنات ذكية حية بمرشحات ذكية وتحويلات وانحناءات غير ضائعة، وتحديد متعدد الطبقات مع المحاذاة والتوزيع والربط، وقنوات ألفا والقناع السريع، و27 نمط دمج من Photoshop إضافة إلى ستة تنويعات من Paint.NET، والعتامة والتعبئة، والأقفال، وتسميات الألوان، ومرشحات الطبقات، والدمج والتسطيح والتحويل إلى صورة نقطية، وطبقة عبر النسخ/القص، واللصق داخل.
    </td>
  </tr>
  <tr>
    <td width="33%" valign="top">
      <h4>🎨 أي لون، بأي عمق</h4>
      مستندات RGB وتدرج رمادي وCMYK وLab بأعماق 8 و16 و32 بت لكل قناة. عمق البت ونموذج اللون بيانات وقت تشغيل، فتعمل كل أداة عند كل عمق.
      <br><br>
      إدارة ألوان ICC حقيقية بلغة Rust خالصة: ملفات تعريف مضمّنة، وإسناد ملف التعريف والتحويل إليه بأهداف التصيير الأربعة كلها مع تعويض النقطة السوداء، ومعاينة طباعية (⌘Y) وتحذير المدى (⇧⌘Y) على وحدة معالجة الرسوميات.
    </td>
    <td width="33%" valign="top">
      <h4>🗂️ الصيغ</h4>
      PSD وPSB، وTIFF طبقي (بيانات طبقات Photoshop داخل ملف TIFF، قراءةً وكتابةً بأي من ترتيبَي البايت)، و<a href="docs/pdn.md">Paint.NET PDN3 ‏(قراءة، بطبقات قابلة للتحرير وبكل أنماط الدمج الـ 14)</a>، و<a href="docs/ora.md">OpenRaster ‏(قراءة وكتابة، بطبقات ومجموعات وأنماط دمج، لصالح Krita وMyPaint وGIMP)</a>، وSVG ‏(يُفتح طبقات أشكال، ويُدرج كائناً ذكياً متجهياً)، إضافة إلى الصيغ المسطحة: PNG وJPEG وTIFF وWebP ‏(بضغط ضائع ودون فقد) وGIF وBMP وTGA وICO وQOI وPNM وOpenEXR وRadiance HDR وFITS ‏(صور فلكية: 8/16 بت و32 بت عائمة، رمادية أو RGB، مع الاحتفاظ بالترويسة)، والصيغة الأصلية <code>.pcraft</code>. تحتفظ الصيغ المسطحة بالأعماق التي تدعمها: يحفظ PNG عينات 8/16 بت، ويحفظ TIFF عينات 8/16 بت و32 بت عائمة، ويحفظ OpenEXR العينات نصف العائمة والعائمة؛ أما JPEG وWebP وسائر صيغ 8 بت فتُنزَّل إليها العينات الأعمق مع تحذير عند التصدير. راجع <a href="crates/codecs/README.md#capability-matrix-default-build">مصفوفة قدرات المرمّزات</a> لكل صيغة. صور HEIC من iPhone وMac للقراءة فقط (مضمّنة في الإصدارات الرسمية؛ وخيار <code>--features heif</code> اختياري عند البناء من المصدر). صيغة AVIF للكتابة فقط مع خاصية البناء الاختيارية <code>avif</code>؛ أما فك ترميز AVIF وصيغة JPEG XL فغير متاحَين. مستندات Affinity‏ (<code>.af</code> و<code>.afdesign</code> و<code>.afphoto</code> و<code>.afpub</code>) تُفتح أصلياً، للقراءة فقط، بلوحات رسمها وطبقاتها ومتجهاتها ونصوصها وصورها وأقنعتها؛ وتُدرَج التأثيرات والتعديلات وغيرها من الأجزاء غير المدعومة في تحذير، أما التصدير بصيغة Affinity فغير متاح.
    </td>
    <td width="33%" valign="top">
      <h4>🪄 أساسيات كل يوم</h4>
      نبرة وتباين ولون تلقائيان · تسوية الدرجات · حجم الصورة واللوحة · اقتصاص وتشذيب · إظهار الكل · تحرير › تعبئة وحد · نسخ مدموجاً · لصق في الموضع نفسه · أدلة ومساطر وشبكة والالتقاط · تسجيل الإجراءات وإعادتها · لوحة أوامر (⌘F).
    </td>
  </tr>
</table>

<br>

## PSD بلا مساومة

دعم PSD في PhotoCraft صندوقة مستقلة كُتبت من المواصفة العامة التي تنشرها Adobe، واختُبرت مقابل مجموعة من ملفات العالم الحقيقي.

- **دورات وفية:** فتح مستند ثم إعادة حفظه يُصيَّر النتيجة نفسها في 307 من ملفات مجموعة اختبار psd-tools البالغة 309، و169 من 170 في مجموعتنا المختلطة ag-psd/psd-tools‏ (`crates/io/tests/corpus.rs`؛ نزّل مجموعة psd-tools بـ `cargo xtask corpus --psd-tools`)، وأي شيء لم نُمذجه بعد (كتل خام، وواصفات، وإضافات) يُنقل كما هو بدلاً من إسقاطه. الملف المعاد حفظه ليس مطابقاً بايتاً ببايت لمصدره: يعيد PhotoCraft كتابة موارد الصورة وسجلات الطبقات والصورة المركبة. الصندوقة المستقلة `photocraft-psd` وحدها، وهي تحلل ملفاً وتكتبه دون نموذج المستند، تعيد إنتاج كل ملف قابل للتحليل في المجموعة بايتاً ببايت (`crates/psd/tests/corpus.rs`).
- **بكسلات مطابقة:** حاكم مركِّب يقارن تصييرنا بالصورة المدمجة التي ينتجها Photoshop نفسه، ويغطي استيفاء التدرجات (الكلاسيكي والإدراكي والخطي)، وتأثيرات الطبقات، وحدود الأشكال، والاقتطاع وعتامة التعبئة.
- **المستندات الكبيرة:** PSB، وملفات 16 و32 بت، ومستندات CMYK وLab، كلها تُفتح أصلياً.

## مبني للوكلاء

كل بند قائمة وأداة ومربع حوار ينفّذ أمراً من سجل واحد يضم أكثر من 500 أمر. الواجهة وسطر الأوامر وقناة التحكم بصيغة JSON وخادم MCP كلها تستدعي الأوامر نفسها، فأي شيء تستطيع النقر عليه يستطيعه سكربت أو وكيل ذكاء اصطناعي كذلك.

```sh
# بلا واجهة: افتح، حرّر، احفظ
photocraft-cli run wave.psd \
  --cmd filter.sharpen.smartSharpen     --params '{"amount":80}' \
  --cmd layer.newAdjustmentLayer.curves --params '{"points":[[0,0],[64,48],[192,212],[255,255]]}' \
  --out wave-final.png

# طبِّق قائمة إجراءات واحدة على مجلد من الصور
photocraft-cli batch --actions grade.json --in ./raw --out ./graded

# كل أمر فرعي يشرح نفسه
photocraft-cli batch --help

# دع وكيلاً يقوده عبر MCP (بلا واجهة، أو موصولاً بالتطبيق قيد التشغيل)
photocraft-cli mcp
```

كما يتيح تطبيق سطح المكتب قناة تحكم موثَّقة تقتصر على الارتجاع المحلي (`photocraft --control`) لفحص حالة الواجهة، وقيادة الأدوات بأحداث المؤشر، والتقاط لقطات خارج الشاشة. كل صورة في هذا الملف صُيِّرت بهذه الطريقة. راجع [`docs/control-protocol.md`](docs/control-protocol.md).

## تحت الغطاء

- **المحرك أولاً:** نموذج مستند من بيانات صرفة ومحرك أوامر، تعلوه واجهة egui رفيعة. تُفرض الطبقية وقت البناء عبر 24 صندوقة (crate).
- **مُركِّبان:** مركِّب على المعالج يعمل مرجعاً حاكماً، ومركِّب wgpu يضع اللوحة على وحدة معالجة الرسوميات. ويُختبر كل منهما مقابل الآخر.
- **بلاطات بالنسخ عند الكتابة:** بلاطات متناثرة بمقاس 256² تجعل التراجع رخيصاً واللوحات الضخمة خفيفة، وتُخزَّن خرائط التأثيرات مؤقتاً بحسب حالة كل طبقة.
- **يعمل في المتصفح:** المحرك والواجهة بأكملهما يُصرَّفان إلى WebAssembly.
- **غرفة نظيفة:** نُفِّذ من المواصفات العامة والسلوك المُلاحَظ فقط. لا شيفرة مغلقة، ولا مظلِّلات، ولا أصول مملوكة.
- **مُختبَر:** أكثر من 1,700 اختبار، تشمل دورات PSD الكاملة، ومولدات اصطناعية، وحُكّام المركِّب، وفحوصات الأعماق المتعددة.

## البدء السريع

```sh
git clone https://github.com/storytold/photocraft
cd photocraft
cargo run --release -p photocraft -- image.psd   # تطبيق سطح المكتب
cargo test --workspace                           # مجموعة الاختبارات
```

تأتي خطوط اليابانية للواجهة وأداة النص من [craft-fonts](https://github.com/storytold/craft-fonts)، وهو مُدخل بناء اختياري (تُضمّه إصدارات سطح المكتب الرسمية دائماً). ومن دونه يستخدم PhotoCraft خطوط CJK المتوفرة لدى نظامك:

```sh
git clone https://github.com/storytold/craft-fonts ../craft-fonts
CRAFT_FONTS_DIR="$PWD/../craft-fonts" cargo run --release -p photocraft
```

للمساهمين الجدد ووكلاء الذكاء الاصطناعي: ابدؤوا بـ [`AGENTS.md`](AGENTS.md) ثم [`docs/`](docs/).

تُرفَق مثبّتات macOS وWindows وLinux وFreeBSD والويب بكل [إصدار على GitHub](https://github.com/storytold/photocraft/releases). وعلى Linux يمكنك اختيار AppImage أو `.deb` أو `.rpm` أو حزمة tar أو حزمة Flatpak. تحتاج حزمة Flatpak إلى بيئة تشغيل freedesktop من [Flathub](https://flathub.org/setup)، ويعرض `flatpak` تثبيتها معها:

```sh
flatpak install --user photocraft-<version>-linux-x86_64.flatpak   # أو -linux-aarch64
flatpak run ai.storyteller.photocraft
```

لا يحتاج AppImage إلى تثبيت: يسجّل التشغيل الأول أيقونة مشغّله وبنده في قائمة التطبيقات ضمن `~/.local/share`، فيعرض شريط التطبيقات أيقونة PhotoCraft على Wayland. اضبط `PHOTOCRAFT_NO_DESKTOP_INTEGRATION=1` لتجاوز ذلك، وراجع [`docs/releasing.md`](docs/releasing.md) › Linux للتراجع عنه.

في جلسة Wayland، لا تُفتح الملفات المُفلات على النافذة بعد: مكتبة winit 0.30، مكتبة النوافذ التي تقوم عليها egui، لا تدعم السحب والإفلات في Wayland‏ (#386). استخدم ملف › فتح، أو انسخ الصورة في مدير الملفات والصقها بـ Ctrl+V. وللسحب والإفلات، شغّل PhotoCraft تحت XWayland:‏ ‏`WAYLAND_DISPLAY= photocraft` أو `WAYLAND_DISPLAY= ./photocraft-<version>-linux-x86_64.AppImage` أو `flatpak run --nosocket=wayland --socket=x11 ai.storyteller.photocraft`. وعلى Wayland يعرض PhotoCraft أمر تثبيتك في تنبيه حتى تتجاهله.

على macOS تأتي أداة سطر الأوامر بصيغة `photocraft-cli-<version>-macos-universal.zip`. الملف التنفيذي موقَّع بمعرّف المطوّر نفسه الذي يستخدمه التطبيق وموثَّق لدى Apple. لا يستطيع ملف تنفيذي مجرد أن يحمل تذكرة توثيق مثبَّتة كما يفعل ملف DMG، لذا يفحص macOS التوثيق عبر الإنترنت في المرة الأولى التي تشغّله فيها. ويمكنك التحقق من ذلك بنفسك:

```sh
ditto -x -k photocraft-cli-<version>-macos-universal.zip .
spctl --assess --type install -vv photocraft-cli-<version>-macos-universal/photocraft-cli
# ... accepted, source=Notarized Developer ID
```

على FreeBSD 14 ‏(x86_64) يتضمن الإصدار حزمة tar مرتَّبة على غرار `/usr/local`. ثبّت مكتبات التشغيل ثم فك ضغط الحزمة هناك:

```sh
pkg install libxkbcommon wayland libX11 libXcursor libXrandr libXi libxcb mesa-libs vulkan-loader gtk3 fontconfig freetype2 alsa-lib
tar -xzf photocraft-<version>-freebsd-x86_64.tar.gz --strip-components 1 -C /usr/local
photocraft
```

للمشرفين: يشرح [`docs/releasing.md`](docs/releasing.md) كيف تُبنى الإصدارات وتُوقَّع وتُنشر.

> [!IMPORTANT]
> **الحالة:** PhotoCraft في مرحلة ألفا مبكرة، ونريد أن نكون صريحين بشأن موقعه الحالي: كثير من ميزات Photoshop موجودة بصورة ما، لكنه **ليس بعد بديلاً عن Photoshop للعمل المهني اليومي**. أكبر الفجوات هي ميزات الذكاء الاصطناعي والتوليد، و15 أداة غائبة، والعمق في التنضيد وسير العمل الاحترافي، وتوافق المكونات الإضافية. كل بند في قوائم Photoshop موصول بأمر ([`docs/parity-checklist.md`](docs/parity-checklist.md))، غير أن ذلك يقيس التوصيلات لا السلوك. الصورة الصادقة بُعداً بُعداً ووجهتنا التالية في [خارطة الطريق](ROADMAP.md) و[تقييم المطابقة](docs/target-app-parity.md). توقّع بعض الخشونة، ونرجو أن تفتحوا الملفات (أرفقوا نظام التشغيل وحجم المستند وعدد الطبقات ولقطة شاشة). ويمكنكم أيضاً إخبارنا بما تعطّل على [Discord](https://discord.gg/artcraft).

## التوثيق

تُصان وثائق المطوّرين والبنية والأتمتة والصيغ والأمان في [كتاب توثيق PhotoCraft](book/).

لاستضافة نسخة المتصفح بنفسك عبر Docker، راجع [استضافة الويب بـ Docker](packaging/web/README.md#docker).

## الأمان

بنية الأمان ونمذجة التهديدات وتقوية المحلِّلات والاختبار الضبابي والإبلاغ عن الثغرات مشروحة في [وثائق الأمان](book/src/security/) و[سياسة أمان المستودع](SECURITY.md).

## مجموعات ملفات الاختبار

يُختبر PhotoCraft مقابل ملفات حقيقية: ملفات PSD حاكمة من تأليفنا في Photoshop ضمن
[photocraft-corpus](https://github.com/storytold/photocraft-corpus)، إضافة إلى مجموعات psd-tools وag-psd وPngSuite،
المثبَّتة إصداراتها والمتحقَّق من بصماتها بـ sha256. نزّلها بـ `cargo xtask corpus --all` وشغّل الاختبارات بـ
`cargo xtask test-corpus` ‏(التفاصيل في [docs/development.md](docs/development.md#test-corpora)).

## التنزيلات

**جديد على PhotoCraft؟** نزّله من [صفحة PhotoCraft على getartcraft.com](https://getartcraft.com/apps/photocraft). فهذه أسهل طريقة لتثبيته.

**تبحث عن بنية أو صيغة بعينها؟** على GitHub يضم [أحدث إصدار](https://github.com/storytold/photocraft/releases/latest) كل البُنى المذكورة أدناه، وتضم [كل الإصدارات](https://github.com/storytold/photocraft/releases) الإصدارات السابقة وملاحظاتها. `<ver>` في أسماء الملفات هو رقم الإصدار، ويسرد `SHA256SUMS.txt` بصمة تحقق لكل ملف.

### Windows

| البنية | المُثبِّت | النسخة المحمولة |
|---|---|---|
| x64 ‏(Intel/AMD بمعمارية 64 بت) | `photocraft-<ver>-windows-x64.msi` | `photocraft-<ver>-windows-x64-portable.zip` |
| arm64 ‏(Snapdragon وحواسيب ARM الأخرى) | `photocraft-<ver>-windows-arm64.msi` | `photocraft-<ver>-windows-arm64-portable.zip` |
| x86 ‏(32 بت) | `photocraft-<ver>-windows-x86.msi` | `photocraft-<ver>-windows-x86-portable.zip` |

المثبّتات والملفات التنفيذية موقَّعة رقمياً.

### macOS

| البنية | الملف | ملاحظات |
|---|---|---|
| التطبيق، شامل (Apple silicon + Intel) | `photocraft-<ver>-macos-universal.dmg` | موقَّع وموثَّق لدى Apple |
| أداة سطر الأوامر، شاملة | `photocraft-cli-<ver>-macos-universal.zip` | موقَّعة وموثَّقة لدى Apple |

### Linux

| الصيغة | x86_64 | aarch64 ‏(ARM64) | ملاحظات |
|---|---|---|---|
| AppImage | `photocraft-<ver>-linux-x86_64.AppImage` | `photocraft-<ver>-linux-aarch64.AppImage` | يعمل في أي مكان؛ يحدّث نفسه عبر [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) ‏(ملفات `.zsync`) |
| Flatpak | `photocraft-<ver>-linux-x86_64.flatpak` | `photocraft-<ver>-linux-aarch64.flatpak` | بيئة معزولة؛ ‏`flatpak install --user <file>` |
| Debian/Ubuntu | `photocraft-<ver>-linux-x86_64.deb` | `photocraft-<ver>-linux-aarch64.deb` | |
| Fedora/RHEL/openSUSE | `photocraft-<ver>-linux-x86_64.rpm` | `photocraft-<ver>-linux-aarch64.rpm` | |
| حزمة tar | `photocraft-<ver>-linux-x86_64.tar.gz` | `photocraft-<ver>-linux-aarch64.tar.gz` | فك الضغط في أي مكان |

‏RISC-V ‏(riscv64): ‏`photocraft-<ver>-linux-riscv64.tar.gz` فقط، مبنية على Ubuntu 24.04 ‏(تتطلب glibc 2.39+).

### FreeBSD

| البنية | الملف |
|---|---|
| x86_64 | `photocraft-<ver>-freebsd-x86_64.tar.gz` |

### الويب (WebAssembly)

| البنية | الملف | ملاحظات |
|---|---|---|
| موقع ثابت | `photocraft-web-<ver>.zip` | يعمل في متصفح حديث؛ استضفه على أي خادم ثابت |

## Crafting Apps

‏PhotoCraft واحد من **Crafting Apps**: أدوات إبداعية مجانية ومفتوحة المصدر من فريق
[ArtCraft](https://getartcraft.com/)، كل واحدة منها مكتوبة بلغة Rust من الصفر، وكل واحدة قادرة على
الوقوف وحدها.

| | التطبيق | الغرض | الشيفرة | اعرف المزيد |
|:-:|---|---|---|---|
| <img src="https://raw.githubusercontent.com/storytold/photocraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.photocraft.png" alt="" width="32" height="32"> | **PhotoCraft** | **معالجة الصور: طبقات وأقنعة ونصوص وملفات PSD حقيقية · أنت هنا** | [GitHub](https://github.com/storytold/photocraft) | [الموقع](https://getartcraft.com/apps/photocraft) |
| <img src="https://raw.githubusercontent.com/storytold/vectorcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png" alt="" width="32" height="32"> | **VectorCraft** | الرسم المتجه | [GitHub](https://github.com/storytold/vectorcraft) | [الموقع](https://getartcraft.com/apps/vectorcraft) |
| <img src="https://raw.githubusercontent.com/storytold/filmcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.filmcraft.png" alt="" width="32" height="32"> | **FilmCraft** | تحرير الفيديو واللون والصوت | [GitHub](https://github.com/storytold/filmcraft) | [الموقع](https://getartcraft.com/apps/filmcraft) |
| <img src="https://raw.githubusercontent.com/storytold/lightcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.lightcraft.png" alt="" width="32" height="32"> | **LightCraft** | مكتبة الصور وتظهير ملفات raw | [GitHub](https://github.com/storytold/lightcraft) | [الموقع](https://getartcraft.com/apps/lightcraft) |
| <img src="https://raw.githubusercontent.com/storytold/pdfcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.pdfcraft.png" alt="" width="32" height="32"> | **PdfCraft** | قراءة ملفات PDF وتنظيمها وحمايتها | [GitHub](https://github.com/storytold/pdfcraft) | [الموقع](https://getartcraft.com/apps/pdfcraft) |
| <img src="https://raw.githubusercontent.com/storytold/effectcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.effectcraft.png" alt="" width="32" height="32"> | **EffectCraft** | الرسوم المتحركة والتأثيرات البصرية | [GitHub](https://github.com/storytold/effectcraft) | [الموقع](https://getartcraft.com/apps/effectcraft) |
| <img src="https://raw.githubusercontent.com/storytold/designcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.designcraft.png" alt="" width="32" height="32"> | **DesignCraft** | تخطيط الصفحات والنشر | [GitHub](https://github.com/storytold/designcraft) | [الموقع](https://getartcraft.com/apps/designcraft) |

وكذلك [**ArtCraft**](https://getartcraft.com/) نفسه، استوديو الصور والفيديو بالذكاء الاصطناعي لدينا، للفنانين الذين يريدون تحكماً حقيقياً.

تشترك Crafting Apps في القواعد نفسها: غرفة نظيفة وRust خالصة، وأصلية على macOS وWindows وLinux، وفي المتصفح عبر WebAssembly، وقابلة للقيادة بالكامل من الوكلاء.

<br>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="انضم إلى مجتمع ArtCraft على Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<h3 align="center">تعال اصنع الأشياء معنا</h3>

<p align="center">
  ‏Discord عندنا هو ملتقى صنّاع الفنون على اختلافهم: من يرسمون، ويصوّرون، وينحتون، ويحرّرون الأفلام،
  وينضدون الحروف، ومن ما زالوا يكتشفون ما يحبون صنعه. شاركنا ما تعمل عليه، واطلب العون،
  وأخبرنا بما تعطّل، أو بما تتمنى أن تفعله هذه الأدوات.
  مهما كان وسيطك ومهما طالت تجربتك، فمكانك بيننا.
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><b>discord.gg/artcraft</b></a> ·
  <a href="https://getartcraft.com/">getartcraft.com</a> ·
  <a href="https://getartcraft.com/apps">Crafting Apps</a> ·
  <a href="https://getartcraft.com/apps/photocraft">PhotoCraft</a>
</p>

---

## الترخيص والإسنادات

يُرخَّص PhotoCraft ترخيصاً مزدوجاً بموجب [MIT](LICENSE-MIT) أو [Apache-2.0](LICENSE-APACHE)، أيهما تختار.
حقوق النشر (c) 2026 محفوظة لفريق ArtCraft ومساهمي PhotoCraft. الإشعارات المطلوبة في [NOTICE](NOTICE).

تحتفظ الخطوط والأيقونات والصور وسائر الأصول المضمّنة بتراخيصها المفتوحة الخاصة بها؛ وكل واحد منها مدرج
باسم مؤلفه ومصدره وترخيصه في [ATTRIBUTION.md](ATTRIBUTION.md).

كل عمل فني معروض هنا يقع في الملك العام (Wikimedia Commons وNASA وU.S. National Archives)؛ والمصادر مدرجة في
[`docs/images/SOURCES.md`](docs/images/SOURCES.md).

إن اسم ArtCraft وشعاره الكتابي وشعاراته في [`docs/brand/`](docs/brand/) علامات تجارية لفريق
ArtCraft وهي خارج نطاق هذا الترخيص. ولا يجوز استخدامها إلا دون تعديل، وفقط كجزء
من هذا المستودع ومن PhotoCraft، بموجب [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt).
وعلى النسخ المشتقة والنسخ المعدّلة إزالتها.

<sub>إن Adobe وPhotoshop وIllustrator وPremiere Pro وLightroom وAcrobat وAfter Effects وInDesign علامات تجارية أو علامات تجارية مسجلة لشركة Adobe Inc. في الولايات المتحدة و/أو بلدان أخرى. وPhotoCraft مشروع مستقل مفتوح المصدر لا يتبع Adobe Inc. ولا تتكفّل به ولا تدعمه؛ ولا تُستخدم هذه الأسماء إلا لوصف سير العمل المتوافق معه.</sub>

<p align="center">
  <a href="https://getartcraft.com/"><img alt="ArtCraft" src="docs/brand/artcraft-mark.svg" width="28"></a><br>
  <sub>من إنتاج فريق <a href="https://getartcraft.com/">ArtCraft</a> ومجتمعه.</sub>
</p>

## تاريخ النجوم

[![مخطط تاريخ النجوم](https://api.star-history.com/svg?repos=storytold/photocraft&type=Date&legend=top-left)](https://www.star-history.com/?repos=storytold%2Fphotocraft&type=date&legend=top-left)

</div>
