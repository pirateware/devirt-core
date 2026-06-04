"use strict";
(function() {
	function fn1(var1, var2, var3, var4, var5, var6, var7) {
		try {
			var var8 = var1[var6](var7), var9 = var8.value;
		} catch (var10) {
			var3(var10);
			return;
		}
		var8.done ? var2(var9) : Promise.resolve(var9).then(var4, var5);
	}
	function fn2(var11) {
		return function() {
			var var12 = this, var13 = arguments;
			return new Promise(function(var14, var15) {
				var var16 = var11.apply(var12, var13);
				function fn3(var17) {
					fn1(var16, var14, var15, fn3, fn4, "next", var17);
				}
				function fn4(var18) {
					fn1(var16, var14, var15, fn3, fn4, "throw", var18);
				}
				fn3(undefined);
			});
		};
	}
	function fn5(var19, var20, var21) {
		var20 in var19 ? Object.defineProperty(var19, var20, {
			value: var21,
			enumerable: true,
			configurable: true,
			writable: true
		}) : var19[var20] = var21;
		return var19;
	}
	function instanceOf(var22, var23) {
		"@swc/helpers - instanceof";
		return var23 != null && typeof Symbol != "undefined" && var23[Symbol.hasInstance] ? !!var23[Symbol.hasInstance](var22) : var22 instanceof var23;
	}
	function fn6(var24) {
		for (var num = 1; num < arguments.length; num++) {
			var var25 = arguments[num] != null ? arguments[num] : {}, list = Object.keys(var25);
			typeof Object.getOwnPropertySymbols == "function" && (list = list.concat(Object.getOwnPropertySymbols(var25).filter(function(var26) {
				return Object.getOwnPropertyDescriptor(var25, var26).enumerable;
			})));
			list.forEach(function(var27) {
				fn5(var24, var27, var25[var27]);
			});
		}
		return var24;
	}
	function fn7(var28, var29) {
		var list1 = Object.keys(var28);
		if (Object.getOwnPropertySymbols) {
			var list2 = Object.getOwnPropertySymbols(var28);
			var29 && (list2 = list2.filter(function(var30) {
				return Object.getOwnPropertyDescriptor(var28, var30).enumerable;
			}));
			list1.push.apply(list1, list2);
		}
		return list1;
	}
	function fn8(var31, var32) {
		var32 = var32 != null ? var32 : {};
		Object.getOwnPropertyDescriptors ? Object.defineProperties(var31, Object.getOwnPropertyDescriptors(var32)) : fn7(Object(var32)).forEach(function(var33) {
			Object.defineProperty(var31, var33, Object.getOwnPropertyDescriptor(var32, var33));
		});
		return var31;
	}
	function fn9(var34) {
		if (Array.isArray(var34)) return var34;
	}
	function fn10(var35, var36) {
		var var37 = var35 == null ? null : typeof Symbol != "undefined" && var35[Symbol.iterator] || var35["@@iterator"];
		if (var37 != null) {
			var arr = [], flag = true, flag1 = false, var38, var39;
			try {
				for (var37 = var37.call(var35); !(flag = (var38 = var37.next()).done) && (arr.push(var38.value), !(var36 && arr.length === var36)); flag = true);
			} catch (var40) {
				flag1 = true;
				var39 = var40;
			} finally {
				try {
					!flag && var37.return != null && var37.return();
				} finally {
					if (flag1) throw var39;
				}
			}
			return arr;
		}
	}
	function fn11() {
		throw new TypeError("Invalid attempt to destructure non-iterable instance.\nIn order to be iterable, non-array objects must have a [Symbol.iterator]() method.");
	}
	function fn12(list3, var41) {
		(var41 == null || var41 > list3.length) && (var41 = list3.length);
		for (var num1 = 0, arr1 = new Array(var41); num1 < var41; num1++) arr1[num1] = list3[num1];
		return arr1;
	}
	function fn13(var42, var43) {
		if (var42) {
			if (typeof var42 == "string") return fn12(var42, var43);
			var var44 = Object.prototype.toString.call(var42).slice(8, -1);
			if (var44 === "Object" && var42.constructor && (var44 = var42.constructor.name), var44 === "Map" || var44 === "Set") return Array.from(var44);
			if (var44 === "Arguments" || /^(?:Ui|I)nt(?:8|16|32)(?:Clamped)?Array$/.test(var44)) return fn12(var42, var43);
		}
	}
	function or1(var45, var46) {
		return fn9(var45) || fn10(var45, var46) || fn13(var45, var46) || fn11();
	}
	function typeOf(var47) {
		"@swc/helpers - typeof";
		return var47 && typeof Symbol != "undefined" && var47.constructor === Symbol ? "symbol" : typeof var47;
	}
	function fn14(var48, var49) {
		var var50, var51, var52, obj = {
			label: 0,
			sent: function() {
				if (var52[0] & 1) throw var52[1];
				return var52[1];
			},
			trys: [],
			ops: []
		}, var53 = Object.create((typeof Iterator == "function" ? Iterator : Object).prototype), var54 = Object.defineProperty;
		var54(var53, "next", { value: fn15(0) });
		var54(var53, "throw", { value: fn15(1) });
		var54(var53, "return", { value: fn15(2) });
		typeof Symbol == "function" && var54(var53, Symbol.iterator, { value: function() {
			return this;
		} });
		return var53;
		function fn15(var55) {
			return function(var56) {
				return fn16([var55, var56]);
			};
		}
		function fn16(var57) {
			if (var50) throw new TypeError("Generator is already executing.");
			for (; var53 && (var53 = 0, var57[0] && (obj = 0)), obj;) try {
				if (var50 = 1, var51 && (var52 = var57[0] & 2 ? var51.return : var57[0] ? var51.throw || ((var52 = var51.return) && var52.call(var51), 0) : var51.next) && !(var52 = var52.call(var51, var57[1])).done) return var52;
				switch (var51 = 0, var52 && (var57 = [var57[0] & 2, var52.value]), var57[0]) {
					case 0:
					case 1:
						var52 = var57;
						break;
					case 4:
						obj.label++;
						return {
							value: var57[1],
							done: false
						};
					case 5:
						obj.label++;
						var51 = var57[1];
						var57 = [0];
						continue;
					case 7:
						var57 = obj.ops.pop();
						obj.trys.pop();
						continue;
					default:
						if (var52 = obj.trys, !(var52 = var52.length > 0 && var52[var52.length - 1]) && (var57[0] === 6 || var57[0] === 2)) {
							obj = 0;
							continue;
						}
						if (var57[0] === 3 && (!var52 || var57[1] > var52[0] && var57[1] < var52[3])) {
							obj.label = var57[1];
							break;
						}
						if (var57[0] === 6 && obj.label < var52[1]) {
							obj.label = var52[1];
							var52 = var57;
							break;
						}
						if (var52 && obj.label < var52[2]) {
							obj.label = var52[2];
							obj.ops.push(var57);
							break;
						}
						var52[2] && obj.ops.pop();
						obj.trys.pop();
						continue;
				}
				var57 = var49.call(var48, obj);
			} catch (var58) {
				var57 = [6, var58];
				var51 = 0;
			} finally {
				var50 = var52 = 0;
			}
			if (var57[0] & 5) throw var57[1];
			return {
				value: var57[0] ? var57[1] : undefined,
				done: true
			};
		}
	}
	var num2 = 300, num3 = 10, num4 = 200100, num5 = 200500, num6 = 300020, num7 = 300030, num8 = 300031;
	function fn17(var59) {
		var obj1 = new URLSearchParams();
		if (var59.params.botnet && obj1.set("botnet", "true"), var59.params._debugSitekeyOverrides && (var59.params._debugSitekeyOverrides.offlabel !== "default" && obj1.set("offlabel", var59.params._debugSitekeyOverrides.offlabel), var59.params._debugSitekeyOverrides.clearanceLevel !== "default" && obj1.set("clearance_level", var59.params._debugSitekeyOverrides.clearanceLevel), var59.params._debugSitekeyOverrides.offlabelShowPrivacy && var59.params._debugSitekeyOverrides.offlabelShowPrivacy !== "default" && obj1.set("offlabel_show_privacy", var59.params._debugSitekeyOverrides.offlabelShowPrivacy), var59.params._debugSitekeyOverrides.offlabelShowHelp && var59.params._debugSitekeyOverrides.offlabelShowHelp !== "default" && obj1.set("offlabel_show_help", var59.params._debugSitekeyOverrides.offlabelShowHelp)), var59.params._debugForceLoopFeedback && obj1.set("force_loop_feedback", "true"), var59.params["offlabel-show-privacy"] !== undefined && obj1.set("offlabel_show_privacy", String(var59.params["offlabel-show-privacy"])), var59.params["offlabel-show-help"] !== undefined && obj1.set("offlabel_show_help", String(var59.params["offlabel-show-help"])), window.__cfDebugTurnstileOutcome && obj1.set("__cfDebugTurnstileOutcome", String(window.__cfDebugTurnstileOutcome)), !(obj1.size === 0 || obj1.toString() === "")) return obj1.toString();
	}
	var str = "cf-chl-widget-", str1 = "cloudflare-challenge", str2 = ".cf-turnstile", str3 = ".cf-challenge", str4 = ".g-recaptcha", str5 = "cf_challenge_response", str6 = "cf-turnstile-response", str7 = "g-recaptcha-response", num9 = 3e4, num10 = 18e4, num11 = 1e4, num12 = 8e3, num13 = 108e5, str8 = "private-token", num14 = 3, num15 = 500, num16 = 500, str9 = "", str10 = "_cftscs_", num17 = 512;
	var var60 = (function(var61) {
		var61.Managed = "managed";
		var61.NonInteractive = "non-interactive";
		var61.Invisible = "invisible";
		return var61;
	})({}), var62 = (function(var63) {
		var63.Normal = "normal";
		var63.Compact = "compact";
		var63.Invisible = "invisible";
		var63.Flexible = "flexible";
		return var63;
	})({}), var64 = (function(var65) {
		var65.Auto = "auto";
		var65.Light = "light";
		var65.Dark = "dark";
		return var65;
	})({}), var66 = (function(var67) {
		var67.Verifying = "verifying";
		var67.VerifyingHavingTroubles = "verifying-having-troubles";
		var67.VerifyingOverrun = "verifying-overrun";
		var67.FailureWoHavingTroubles = "failure-wo-having-troubles";
		var67.FailureHavingTroubles = "failure-having-troubles";
		var67.FailureFeedback = "failure-feedback";
		var67.FailureFeedbackCode = "failure-feedback-code";
		var67.ExpiredNeverRefresh = "expired-never-refresh";
		var67.ExpiredManualRefresh = "expired-manual-refresh";
		var67.TimeoutNeverRefresh = "timeout-never-refresh";
		var67.TimeoutManualRefresh = "timeout-manual-refresh";
		var67.InteractivityRequired = "interactivity-required";
		var67.UnsupportedBrowser = "unsupported-browser";
		var67.TimeCheckCachedWarning = "time-check-cached-warning";
		var67.InvalidDomain = "invalid-domain";
		return var67;
	})({}), var68 = (function(var69) {
		var69.Never = "never";
		var69.Auto = "auto";
		return var69;
	})({}), var70 = (function(var71) {
		var71.Never = "never";
		var71.Manual = "manual";
		var71.Auto = "auto";
		return var71;
	})({}), var72 = (function(var73) {
		var73.Never = "never";
		var73.Manual = "manual";
		var73.Auto = "auto";
		return var73;
	})({}), var74 = (function(var75) {
		var75.Always = "always";
		var75.Execute = "execute";
		var75.InteractionOnly = "interaction-only";
		return var75;
	})({}), var76 = (function(var77) {
		var77.Render = "render";
		var77.Execute = "execute";
		return var77;
	})({}), var78 = (function(var79) {
		var79.Execute = "execute";
		return var79;
	})({}), var80 = (function(var81) {
		var81.New = "new";
		var81.CrashedRetry = "crashed_retry";
		var81.FailureRetry = "failure_retry";
		var81.StaleExecute = "stale_execute";
		var81.AutoExpire = "auto_expire";
		var81.AutoTimeout = "auto_timeout";
		var81.ManualRefresh = "manual_refresh";
		var81.Api = "api";
		var81.CheckDelays = "check_delays";
		var81.UpgradeReload = "upgrade_reload";
		var81.TimeCheckCachedWarningAux = "time_check_cached_warning_aux";
		var81.JsCookiesMissingAux = "js_cookies_missing_aux";
		var81.RedirectingTextOverrun = "redirecting_text_overrun";
		return var81;
	})({});
	var arr2 = [
		"bg-bg",
		"da-dk",
		"de-de",
		"el-gr",
		"ja-jp",
		"ms-my",
		"ru-ru",
		"sk-sk",
		"sl-si",
		"sr-ba",
		"tl-ph",
		"uk-ua"
	], arr3 = [
		"ar-eg",
		"es-es",
		"cs-cz",
		"fa-ir",
		"fr-fr",
		"hr-hr",
		"hu-hu",
		"id-id",
		"it-it",
		"lv-lv",
		"nb-no",
		"nl-nl",
		"pl-pl",
		"pt-br",
		"th-th",
		"tr-tr",
		"ro-ro"
	], arr4 = [
		"https://challenges.cloudflare.com",
		"https://challenges-staging.cloudflare.com",
		"https://challenges.fed.cloudflare.com"
	];
	function fn18(var82, var83, var84) {
		var var85, var86 = (var85 = var84 == null ? undefined : var84.origin) !== null && var85 !== undefined ? var85 : "https://challenges.cloudflare.com";
		if (var83) {
			var var87;
			return (var87 = var82["base-url"]) !== null && var87 !== undefined ? var87 : var86;
		}
		return var86;
	}
	function call(var88, var89, var90, var91, var92, var93, var94, var95, var96) {
		var var97 = fn18(var90, var92, var95), var98 = var93 ? "h/".concat(var93, "/") : "", var99 = var96 ? "&".concat(var96) : "", var100 = var90["feedback-enabled"] === false ? "fbD" : "fbE", var101 = var90.chlPageOfflabel ? "&offlabel=true" : "";
		return "".concat(var97, "/cdn-cgi/challenge-platform/").concat(var98, "turnstile/f/ov2/av0/rch").concat(var91, "/").concat(var88, "/").concat(var89, "/").concat(var90.theme, "/").concat(var100, "/").concat(var94, "/").concat(var90.size, "?lang=").concat(var90.language).concat(var101).concat(var99);
	}
	var pick = function(var102) {
		var var103, var104, var105, var106, var107 = window.innerWidth < 400, var108 = var102.state !== var66.FailureFeedbackCode && (var102.state === var66.FailureFeedback || var102.state === var66.FailureHavingTroubles || !var102.errorCode), var109 = arr2.indexOf((var103 = (var105 = var102.displayLanguage) === null || var105 === undefined ? undefined : var105.toLowerCase()) !== null && var103 !== undefined ? var103 : "nonexistent") !== -1, var110 = arr3.indexOf((var104 = (var106 = var102.displayLanguage) === null || var106 === undefined ? undefined : var106.toLowerCase()) !== null && var104 !== undefined ? var104 : "nonexistent") !== -1;
		return var107 ? pick1({
			isModeratelyVerbose: var110,
			isSmallerFeedback: var108,
			isVerboseLanguage: var109
		}) : var108 && var109 ? "680px" : var108 && var110 ? "670px" : var108 ? "650px" : var109 ? "690px" : "680px";
	}, pick1 = function(var111) {
		var var112 = var111.isVerboseLanguage, var113 = var111.isSmallerFeedback, var114 = var111.isModeratelyVerbose;
		return var113 && var112 ? "660px" : var113 && var114 ? "620px" : var113 ? "600px" : var112 ? "770px" : var114 ? "740px" : "730px";
	};
	function call1(var115) {
		call1 = Object.setPrototypeOf ? Object.getPrototypeOf : function(var116) {
			return var116.__proto__ || Object.getPrototypeOf(var116);
		};
		return call1(var115);
	}
	function call2() {
		try {
			var var117 = !Boolean.prototype.valueOf.call(Reflect.construct(Boolean, [], function() {}));
		} catch (var118) {}
		return (call2 = function() {
			return !!var117;
		})();
	}
	function fn19(var119) {
		if (var119 === undefined) throw new ReferenceError("this hasn't been initialised - super() hasn't been called");
		return var119;
	}
	function pick2(var120, var121) {
		return var121 && (typeOf(var121) === "object" || typeof var121 == "function") ? var121 : fn19(var120);
	}
	function call3(var122, var123, var124) {
		var123 = call1(var123);
		return pick2(var122, Reflect.construct(var123, var124 || [], call1(var122).constructor));
	}
	function fn20(var125, var126) {
		if (!instanceOf(var125, var126)) throw new TypeError("Cannot call a class as a function");
	}
	function call4(var127, var128) {
		call4 = Object.setPrototypeOf || function(var129, var130) {
			var129.__proto__ = var130;
			return var129;
		};
		return call4(var127, var128);
	}
	function fn21(var131, var132) {
		if (typeof var132 != "function" && var132 !== null) throw new TypeError("Super expression must either be null or a function");
		var131.prototype = Object.create(var132 && var132.prototype, { constructor: {
			value: var131,
			writable: true,
			configurable: true
		} });
		var132 && call4(var131, var132);
	}
	function call5(var133, var134, var135) {
		call5 = Reflect.construct;
		return call5.apply(null, arguments);
	}
	function notEqual(var139) {
		return Function.toString.call(var139).indexOf("[native code]") !== -1;
	}
	function call6(var140) {
		var var141 = typeof Map == "function" ? new Map() : undefined;
		call6 = function(var142) {
			if (var142 === null || !notEqual(var142)) return var142;
			if (typeof var142 != "function") throw new TypeError("Super expression must either be null or a function");
			if (typeof var141 != "undefined") {
				if (var141.has(var142)) return var141.get(var142);
				var141.set(var142, call7);
			}
			function call7() {
				return call5(var142, arguments, call1(this).constructor);
			}
			call7.prototype = Object.create(var142.prototype, { constructor: {
				value: call7,
				enumerable: false,
				writable: true,
				configurable: true
			} });
			return call4(call7, var142);
		};
		return call6(var140);
	}
	var var143 = (function(var144) {
		"use strict";
		fn21(fn22, var144);
		function fn22(var145, var146) {
			fn20(this, fn22);
			var var147;
			var147 = call3(this, fn22, [var145]);
			fn5(var147, "code", undefined);
			var147.name = "TurnstileError";
			var147.code = var146;
			return var147;
		}
		return fn22;
	})(call6(Error));
	var regex = /^https:\/\/challenges(?:\.fed)?\.cloudflare\.com\/turnstile\/v0(\/.*)?\/api\.js/;
	function fn23(var148, var149) {
		throw new var143("[Cloudflare Turnstile] ".concat(var148, "."), var149);
	}
	function fn24(var150) {
		console.warn("[Cloudflare Turnstile] ".concat(var150));
	}
	function pick3(var151) {
		return var151.startsWith(str) ? var151.slice(str.length) : null;
	}
	function call8(var152) {
		return "".concat(str).concat(var152);
	}
	function fn25(var153, var154) {
		var flag2 = true, flag3 = false, var155 = undefined;
		try {
			for (var var156 = Object.getOwnPropertySymbols(var153)[Symbol.iterator](), var157; !(flag2 = (var157 = var156.next()).done); flag2 = true) {
				var var158 = var157.value, var159, var160 = (var159 = Object.getOwnPropertyDescriptor(var153, var158)) === null || var159 === undefined ? undefined : var159.value;
				if (var154(var160)) return var160;
			}
		} catch (var161) {
			flag3 = true;
			var155 = var161;
		} finally {
			try {
				!flag2 && var156.return != null && var156.return();
			} finally {
				if (flag3) throw var155;
			}
		}
	}
	function fn26() {
		var var162 = regex, var163 = document.currentScript;
		if (instanceOf(var163, HTMLScriptElement) && var162.test(var163.src)) return var163;
		for (var elems = document.querySelectorAll("script"), num18 = 0, var164; var164 = elems[num18]; num18++) if (instanceOf(var164, HTMLScriptElement) && var162.test(var164.src)) return var164;
	}
	function fn27() {
		var var165 = fn26();
		var165 || fn23("Could not find Turnstile valid script tag, some features may not be available", 43777);
		var var166 = var165.src, var167;
		try {
			var167 = new URL(var166);
		} catch (var168) {
			fn23("Could not parse Turnstile script tag URL", 43777);
		}
		var obj3 = {
			loadedAsync: false,
			params: new URLSearchParams(),
			src: var166,
			url: var167
		};
		(var165.async || var165.defer) && (obj3.loadedAsync = true);
		var parts = var166.split("?");
		parts.length > 1 && (obj3.params = new URLSearchParams(parts[1]));
		return obj3;
	}
	function fn28(var169) {
		try {
			return new URL(var169, window.location.href).origin;
		} catch (var170) {
			return;
		}
	}
	function fn29(var171, var172, var173) {
		if (!var173) {
			return;
		}
		var171 == null || var171.postMessage(var172, var173);
	}
	function fn30(var174, var175, var176) {
		fn29(var174.contentWindow, var175, var176);
	}
	var num19 = 5e3, str11 = "auto-troubleshoot-click";
	function call9(var177, var178) {
		var var179 = fn18(var177.params, false, var178), var180 = "h/".concat("b", "/");
		return "".concat(var179, "/cdn-cgi/challenge-platform/").concat(var180, "feedback-reports");
	}
	var var181 = function(var182, var183, var184, var185, var186) {
		var var187, var188, var189, var190, var191, var192, var193;
		if (!var185 || !var184) return false;
		var var194 = call9(var182, var186), obj4 = new FormData();
		obj4.append("consent", "on");
		obj4.append("origin", var183);
		obj4.append("issue", str11);
		obj4.append("description", "");
		obj4.append("rayId", var184);
		obj4.append("sitekey", (var187 = var182.params.sitekey) !== null && var187 !== undefined ? var187 : "");
		obj4.append("rcV", (var188 = var182.rcV) !== null && var188 !== undefined ? var188 : "");
		obj4.append("cfChlOut", (var189 = var182.cfChlOut) !== null && var189 !== undefined ? var189 : "");
		obj4.append("cfChlOutS", (var190 = var182.cfChlOutS) !== null && var190 !== undefined ? var190 : "");
		obj4.append("mode", (var191 = var182.mode) !== null && var191 !== undefined ? var191 : "");
		obj4.append("errorCode", String((var192 = var182.errorCode) !== null && var192 !== undefined ? var192 : 0));
		obj4.append("frMd", var185);
		obj4.append("displayLanguage", (var193 = var182.displayLanguage) !== null && var193 !== undefined ? var193 : "");
		try {
			if (typeof navigator != "undefined" && typeof navigator.sendBeacon == "function" && navigator.sendBeacon(var194, obj4)) return true;
		} catch (var195) {
			fn24("auto feedback report: sendBeacon threw synchronously, falling through to fetch (".concat(pick5(var195), ")"));
		}
		try {
			fetch(var194, fn6({
				body: obj4,
				keepalive: true,
				method: "POST",
				mode: "no-cors"
			}, pick4()));
			return true;
		} catch (var196) {
			fn24("auto feedback report: keepalive fetch threw synchronously, falling through to plain fetch (".concat(pick5(var196), ")"));
		}
		try {
			fetch(var194, fn6({
				body: obj4,
				method: "POST",
				mode: "no-cors"
			}, pick4()));
		} catch (var197) {
			fn24("auto feedback report: all transports failed (".concat(pick5(var197), ")"));
		}
		return false;
	};
	function pick4() {
		return typeof AbortSignal == "undefined" || typeof AbortSignal.timeout != "function" ? {} : { signal: AbortSignal.timeout(num19) };
	}
	function pick5(var198) {
		return instanceOf(var198, Error) ? var198.message : "unknown error";
	}
	var var199 = Symbol(), str12 = "host-origin", var200 = function(var201) {
		var201.feedbackPopup && !var201.feedbackPopup.closed && var201.feedbackPopup.close();
		var201.feedbackPopup = undefined;
		var201.feedbackPopupOrigin = undefined;
	};
	function pick6(var202) {
		var var203, var204, var205, var206 = (var205 = document.getElementById(var202)) === null || var205 === undefined || (var204 = var205.parentElement) === null || var204 === undefined || (var203 = var204.parentElement) === null || var203 === undefined ? undefined : var203.parentElement;
		return instanceOf(var206, HTMLDivElement) ? var206 : null;
	}
	function fn31(var207) {
		var var208;
		if (!((typeof var207 == "undefined" ? "undefined" : typeOf(var207)) !== "object" || var207 === null)) {
			var var209 = (var208 = Object.getOwnPropertyDescriptor(var207, "cleanup")) === null || var208 === undefined ? undefined : var208.value;
			return typeof var209 == "function" ? var209 : undefined;
		}
	}
	function fn32(var210) {
		var var211, var212 = fn31((var211 = Object.getOwnPropertyDescriptor(var210, var199)) === null || var211 === undefined ? undefined : var211.value);
		if (var212) return var212;
		var flag4 = true, flag5 = false, var213 = undefined;
		try {
			for (var var214 = Object.getOwnPropertySymbols(var210)[Symbol.iterator](), var215; !(flag4 = (var215 = var214.next()).done); flag4 = true) {
				var var216 = var215.value, var217, var218 = fn31((var217 = Object.getOwnPropertyDescriptor(var210, var216)) === null || var217 === undefined ? undefined : var217.value);
				if (var218) return var218;
			}
		} catch (var219) {
			flag5 = true;
			var213 = var219;
		} finally {
			try {
				!flag4 && var214.return != null && var214.return();
			} finally {
				if (flag5) throw var213;
			}
		}
	}
	function fn33(var220, var221) {
		Object.defineProperty(var220, var199, {
			configurable: true,
			enumerable: false,
			value: { cleanup: var221 }
		});
	}
	function fn34(var222) {
		Reflect.deleteProperty(var222, var199);
	}
	function call10(var223) {
		var obj5 = new URL(var223, window.location.href), obj6 = new URLSearchParams(obj5.hash.startsWith("#") ? obj5.hash.slice(1) : obj5.hash);
		obj6.set(str12, window.location.origin);
		obj5.hash = obj6.toString();
		return obj5.toString();
	}
	var var224 = function(var225, var226, var227, var228) {
		var var229, var230 = var225.endsWith("-fr") ? var225 : "".concat(var225, "-fr"), var231 = fn18(var226.params, false, var228), var232 = "h/".concat("b", "/"), var233 = call10("".concat(var231, "/cdn-cgi/challenge-platform/").concat(var232, "feedback-reports/").concat(pick3(var225), "/").concat(var226.displayLanguage, "/").concat((var229 = var226.params.theme) !== null && var229 !== undefined ? var229 : var226.theme, "/").concat(var227));
		if (var200(var226), window.top !== window.self) {
			var var234 = window.open(var233, "_blank");
			if (var234) {
				var226.feedbackPopupOrigin = fn28(var233);
				var226.feedbackPopup = var234;
				return;
			}
			fn24("Unable to open feedback report popup, falling back to the embedded feedback overlay.");
		}
		var226.wrapper.parentNode || fn23("Cannot initialize Widget, Element not found (#".concat(var225, ")."), 3074);
		var var235 = pick6(var230);
		if (var235) {
			var var236;
			(var236 = fn32(var235)) === null || var236 === undefined || var236();
		}
		var elem = document.createElement("div");
		elem.style.position = "fixed";
		elem.style.zIndex = "2147483646";
		elem.style.width = "100vw";
		elem.style.height = "100vh";
		elem.style.top = "0";
		elem.style.left = "0";
		elem.style.transformOrigin = "center center";
		elem.style.overflowX = "hidden";
		elem.style.overflowY = "auto";
		elem.style.background = "rgba(0,0,0,0.4)";
		var elem1 = document.createElement("div");
		elem1.className = "cf-wrapper-turnstile-feedback";
		elem1.style.display = "table-cell";
		elem1.style.verticalAlign = "middle";
		elem1.style.width = "100vw";
		elem1.style.height = "100vh";
		var elem2 = document.createElement("div");
		elem2.className = "cf-turnstile-feedback";
		elem2.id = "cf-fr-id";
		elem2.style.width = "100vw";
		elem2.style.maxWidth = "500px";
		elem2.style.height = pick(var226);
		elem2.style.position = "relative";
		elem2.style.zIndex = "2147483647";
		elem2.style.backgroundColor = "#ffffff";
		elem2.style.borderRadius = "5px";
		elem2.style.left = "0px";
		elem2.style.top = "0px";
		elem2.style.overflow = "hidden";
		elem2.style.margin = "0px auto";
		var var237 = function() {
			elem2.style.height = pick(var226);
		}, var238 = function() {
			var var239;
			fn34(elem);
			var226.feedbackIframeOrigin = undefined;
			window.removeEventListener("resize", var237);
			(var239 = elem.parentNode) === null || var239 === undefined || var239.removeChild(elem);
		}, elem3 = document.createElement("iframe");
		elem3.id = var230;
		elem3.setAttribute("src", var233);
		elem3.setAttribute("allow", "cross-origin-isolated; fullscreen");
		elem3.setAttribute("sandbox", "allow-same-origin allow-scripts allow-popups allow-forms");
		elem3.setAttribute("scrolling", "yes");
		elem3.style.borderWidth = "0px";
		elem3.style.width = "100%";
		elem3.style.height = "100%";
		elem3.style.overflow = "auto";
		var226.feedbackIframeOrigin = fn28(var233);
		var var240 = document.createElementNS("http://www.w3.org/2000/svg", "svg");
		var240.setAttribute("tabindex", "0");
		var240.setAttribute("role", "img");
		var240.setAttribute("aria-label", "Close button icon");
		var240.style.position = "absolute";
		var240.style.width = "26px";
		var240.style.height = "26px";
		var240.style.zIndex = "2147483647";
		var240.style.cursor = "pointer";
		var226.displayRtl ? var240.style.left = "24px" : var240.style.right = "24px";
		var240.style.top = "24px";
		var240.setAttribute("width", "20");
		var240.setAttribute("height", "20");
		var240.addEventListener("click", function(var241) {
			var241.stopPropagation();
			var238();
		});
		var240.addEventListener("keydown", function(var242) {
			(var242.key === "Enter" || var242.key === " ") && (var242.stopPropagation(), var238());
		});
		var var243 = document.createElementNS("http://www.w3.org/2000/svg", "ellipse");
		var243.setAttribute("ry", "12");
		var243.setAttribute("rx", "12");
		var243.setAttribute("cy", "12");
		var243.setAttribute("cx", "12");
		var243.setAttribute("fill", "none");
		var243.setAttribute("stroke-width", "0");
		var240.appendChild(var243);
		var var244 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		var244.setAttribute("stroke-width", "1");
		var244.setAttribute("fill", "none");
		var244.setAttribute("x1", "6");
		var244.setAttribute("x2", "18");
		var244.setAttribute("y1", "18");
		var244.setAttribute("y2", "5");
		var var245 = document.createElementNS("http://www.w3.org/2000/svg", "line");
		var245.setAttribute("stroke-width", "1");
		var245.setAttribute("fill", "none");
		var245.setAttribute("x1", "6");
		var245.setAttribute("x2", "18");
		var245.setAttribute("y1", "5");
		var245.setAttribute("y2", "18");
		var226.theme === var64.Light ? (var244.setAttribute("stroke", "#0A0A0A"), var245.setAttribute("stroke", "#0A0A0A")) : (var244.setAttribute("stroke", "#F2F2F2"), var245.setAttribute("stroke", "#F2F2F2"));
		var240.appendChild(var244);
		var240.appendChild(var245);
		elem2.appendChild(elem3);
		elem2.appendChild(var240);
		elem1.appendChild(elem2);
		elem.appendChild(elem1);
		elem.addEventListener("click", var238);
		var226.wrapper.parentNode.appendChild(elem);
		window.addEventListener("resize", var237);
		fn33(elem, var238);
	}, var246 = function(var247) {
		var var248, var249 = pick6(var247.endsWith("-fr") ? var247 : "".concat(var247, "-fr"));
		if (var249) {
			var var250 = fn32(var249);
			if (var250) {
				var250();
				return;
			}
			fn24("Unable to find feedback overlay cleanup handler. Removing overlay without cleanup.");
			(var248 = var249.parentNode) === null || var248 === undefined || var248.removeChild(var249);
		}
	};
	function pick7(var251, var252) {
		return var251 ? var252 ? true : arr4.indexOf(var251) !== -1 : false;
	}
	var var253 = (function(var254) {
		var254.Failure = "failure";
		var254.Verifying = "verifying";
		var254.Overruning = "overrunning";
		var254.Custom = "custom";
		var254.TimeCheckCachedWarning = "timecheckcachedwarning";
		var254.UnsupportedBrowser = "unsupportedbrowser";
		var254.InvalidDomain = "invaliddomain";
		return var254;
	})({});
	var pick8 = function(list4) {
		var var255 = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : 3;
		return list4.length > var255 ? list4.slice(0, var255) : list4;
	};
	function call11(var256) {
		if (!var256) return "-";
		var call12 = function(var257, var258) {
			if (!var257 || var257.tagName === "BODY") return var258;
			for (var num20 = 1, var259 = var257.previousElementSibling; var259;) {
				var259.tagName === var257.tagName && num20++;
				var259 = var259.previousElementSibling;
			}
			return call12(var257.parentNode, "/".concat("".concat(pick8(var257.tagName.toLowerCase()), "[").concat(num20, "]")).concat(var258));
		};
		return call12(var256, "");
	}
	function call13(var261) {
		if (!var261) return "";
		var var262 = var261.getBoundingClientRect();
		return "".concat(var262.top, "|").concat(var262.right);
	}
	var obj7 = {
		button: "b",
		checkbox: "c",
		email: "e",
		hidden: "h",
		number: "n",
		password: "p",
		radio: "r",
		select: "sl",
		submit: "s",
		text: "t",
		textarea: "ta"
	};
	function call14(var263) {
		if (!var263) return "";
		var elem4 = var263.closest("form");
		if (!elem4) return "nf";
		var list5 = Array.from(elem4.querySelectorAll("input, select, textarea, button")), var264 = list5.slice(0, 20).map(function(var265) {
			return obj7[var265.type] || "-";
		}).join("");
		return [
			"m:".concat(elem4.getAttribute("method") || ""),
			"f:".concat(list5.length),
			var264
		].join("|");
	}
	function call15(var266, var267, var268) {
		for (var str13 = "", var269 = document.createNodeIterator(var266, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT), var270; (var270 = var269.nextNode()) !== null && str13.length < var268;) {
			for (var num21 = 0, var271 = var270; var271 !== null && var271 !== var266;) {
				num21++;
				var271 = var271.parentNode;
			}
			if (!(num21 > var267)) if (var270.nodeType === Node.ELEMENT_NODE) {
				var var272 = var270;
				str13 += "".concat(pick8(var272.tagName.toLowerCase()));
				var flag6 = true, flag7 = false, var273 = undefined;
				try {
					for (var var274 = Array.from(var272.attributes)[Symbol.iterator](), var275; !(flag6 = (var275 = var274.next()).done); flag6 = true) {
						var var276 = var275.value;
						str13 += "_".concat(pick8(var276.name, 2));
					}
				} catch (var277) {
					flag7 = true;
					var273 = var277;
				} finally {
					try {
						!flag6 && var274.return != null && var274.return();
					} finally {
						if (flag7) throw var273;
					}
				}
				str13 += ">";
			} else var270.nodeType === Node.TEXT_NODE && (str13 += "-t");
		}
		return str13.slice(0, var268);
	}
	function shiftRightU(var278) {
		if (typeof var278 != "string") throw new Error("djb2: expected string, got ".concat(typeof var278 == "undefined" ? "undefined" : typeOf(var278)));
		for (var num22 = 5381, num23 = 0; num23 < var278.length; num23++) {
			num22 = num22 * 33 ^ var278.charCodeAt(num23);
		}
		return num22 >>> 0;
	}
	function pick9() {
		for (var var279 = arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : window, var280 = var279; var280 && var280.top !== var280 && !var280.location.href.startsWith("http");) var280 = var280.top;
		return var280 == null ? undefined : var280.location.href;
	}
	var num24 = 900, num25 = 45, num26 = 50;
	function fn35(var281, var282, var283) {
		var var284 = var281.widgetMap.get(var282);
		(var284 == null ? undefined : var284.retryTimeout) !== undefined && (window.clearTimeout(var284.retryTimeout), var284.retryTimeout = undefined);
		fn24("Cannot find Widget ".concat(var283, ", consider using turnstile.remove() to clean up a widget."));
		var281.widgetMap.delete(var282);
	}
	function fn36(var285) {
		var285.watchCatSeq++;
		var arr6 = [], flag8 = true, flag9 = false, var286 = undefined;
		try {
			for (var var287 = var285.widgetMap[Symbol.iterator](), var288; !(flag8 = (var288 = var287.next()).done); flag8 = true) {
				var var289 = or1(var288.value, 2), var290 = var289[0], var291 = var289[1], var292 = call8(var290);
				if (!var292 || !var291.shadow || !var291.wrapper.isConnected) {
					var291.watchcat.missingWidgetWarning || (var291.watchcat.missingWidgetWarning = true, arr6.push({
						widgetElId: var292,
						widgetId: var290
					}));
					continue;
				}
				var elem5 = var291.shadow.querySelector("#".concat(var292));
				if (!elem5) {
					var291.watchcat.missingWidgetWarning || (var291.watchcat.missingWidgetWarning = true, arr6.push({
						widgetElId: var292,
						widgetId: var290
					}));
					continue;
				}
				if (var291.watchcat.seq = var285.watchCatSeq, var291.watchcat.lastAckedSeq === 0 && (var291.watchcat.lastAckedSeq = var285.watchCatSeq), !(var291.isComplete || var291.isFailed)) {
					var var293 = var291.watchcat.seq - 1 - num25, var294 = var291.watchcat.lastAckedSeq < var293, var295 = var291.watchcat.seq - 1 - num26, var296 = var291.isOverrunning && var291.watchcat.overrunBeginSeq !== 0 && var291.watchcat.overrunBeginSeq < var295;
					if ((var291.isExecuting || !var291.isInitialized || var291.isInitialized && !var291.isStale && !var291.isExecuted) && var291.watchcat.lastAckedSeq !== 0 && var294 || var296) {
						var var297, var298;
						var291.watchcat.lastAckedSeq = 0;
						var291.watchcat.seq = 0;
						var291.isExecuting = false;
						var var299 = function(var300, var301) {
							console.log("Turnstile Widget seem to have ".concat(var300, ": "), var301);
						};
						var299(var294 ? "hung" : "crashed", var290);
						var var302 = var294 ? num7 : num8;
						if ((var298 = var285.internalMsgHandler) === null || var298 === undefined || var298.call(var285, {
							code: var302,
							event: "fail",
							rcV: (var297 = var291.nextRcV) !== null && var297 !== undefined ? var297 : str9,
							source: str1,
							widgetId: var290
						}), 0) var var303;
						continue;
					}
					fn30(elem5, {
						event: "meow",
						seq: var285.watchCatSeq,
						source: str1,
						widgetId: var290
					}, var291.iframeOrigin);
				}
			}
		} catch (var304) {
			flag9 = true;
			var286 = var304;
		} finally {
			try {
				!flag8 && var287.return != null && var287.return();
			} finally {
				if (flag9) throw var286;
			}
		}
		var flag10 = true, flag11 = false, var305 = undefined;
		try {
			for (var var306 = arr6[Symbol.iterator](), var307; !(flag10 = (var307 = var306.next()).done); flag10 = true) {
				var var308 = var307.value, var309 = var308.widgetElId, var310 = var308.widgetId;
				fn35(var285, var310, var309);
			}
		} catch (var311) {
			flag11 = true;
			var305 = var311;
		} finally {
			try {
				!flag10 && var306.return != null && var306.return();
			} finally {
				if (flag11) throw var305;
			}
		}
		arr6.length > 0 && var285.widgetMap.size === 0 && fn38(var285);
	}
	function fn37(var312) {
		var312.watchCatInterval === null && (var312.watchCatInterval = setInterval(function() {
			fn36(var312);
		}, num24));
	}
	function fn38(var313) {
		var var314 = arguments.length > 1 && arguments[1] !== undefined ? arguments[1] : false;
		var313.watchCatInterval !== null && (var313.widgetMap.size === 0 || var314) && (clearInterval(var313.watchCatInterval), var313.watchCatInterval = null);
	}
	var var315 = Symbol();
	function pick10(var316) {
		return (typeof var316 == "undefined" ? "undefined" : typeOf(var316)) === "object" && var316 !== null ? var316 : undefined;
	}
	function and(var317) {
		return (typeof var317 == "undefined" ? "undefined" : typeOf(var317)) === "object" && var317 !== null && "widgetMap" in var317 && instanceOf(var317.widgetMap, Map) && "upgradeAttempts" in var317 && typeof var317.upgradeAttempts == "number" && "upgradeCompletedCount" in var317 && typeof var317.upgradeCompletedCount == "number";
	}
	function fn39(var318, var319) {
		Object.defineProperty(var318, var315, {
			configurable: true,
			enumerable: false,
			value: var319
		});
	}
	function fn40(var320) {
		var var321, var322 = (var321 = Object.getOwnPropertyDescriptor(var320, var315)) === null || var321 === undefined ? undefined : var321.value;
		if (and(var322)) return var322;
		var var323 = fn25(var320, and);
		if (var323) return var323;
	}
	function fn41(var324) {
		Reflect.deleteProperty(var324, var315);
		var flag12 = true, flag13 = false, var325 = undefined;
		try {
			for (var var326 = Object.getOwnPropertySymbols(var324)[Symbol.iterator](), var327; !(flag12 = (var327 = var326.next()).done); flag12 = true) {
				var var328 = var327.value, var329, var330 = (var329 = Object.getOwnPropertyDescriptor(var324, var328)) === null || var329 === undefined ? undefined : var329.value;
				and(var330) && Reflect.deleteProperty(var324, var328);
			}
		} catch (var331) {
			flag13 = true;
			var325 = var331;
		} finally {
			try {
				!flag12 && var326.return != null && var326.return();
			} finally {
				if (flag13) throw var325;
			}
		}
	}
	function pick11(var332) {
		return !Number.isFinite(var332.apiJsReloadBackoffMs) || var332.apiJsReloadBackoffMs <= 0 ? num9 : Math.min(var332.apiJsReloadBackoffMs, num10);
	}
	function pick12(var333) {
		return !Number.isFinite(var333.apiJsReloadNextAllowedTsMs) || var333.apiJsReloadNextAllowedTsMs <= 0 ? 0 : var333.apiJsReloadNextAllowedTsMs;
	}
	function fn42(var334, var335) {
		var var336 = arguments.length > 2 && arguments[2] !== undefined ? arguments[2] : fn26;
		var335.upgradeAttempts++;
		var var337 = var336();
		if (!(var337 != null && var337.parentNode)) return false;
		var var338 = pick10(var334);
		if (!var338) return false;
		var var339 = var337 == null ? undefined : var337.nonce;
		fn39(var338, var335);
		var obj8 = new URL(var337.src), elem6 = document.createElement("script");
		obj8.searchParams.set("_upgrade", "true");
		obj8.searchParams.set("_cb", String(Date.now()));
		elem6.async = true;
		var339 && (elem6.nonce = var339);
		elem6.setAttribute("crossorigin", "anonymous");
		elem6.src = obj8.toString();
		try {
			var337.parentNode.replaceChild(elem6, var337);
			return true;
		} catch (var340) {
			if (!instanceOf(var340, DOMException)) throw var340;
			fn41(var338);
			return false;
		}
	}
	function fn43(var341, var342, var343) {
		var var344, var345, var346 = pick10(var341);
		if (!var346) return false;
		var var347 = fn40(var346);
		if (!var347) return false;
		var var348 = var347.apiJsMismatchReloadPending;
		var342.isReady = var347.isReady;
		var342.isRecaptchaCompatibilityMode = var347.isRecaptchaCompatibilityMode;
		var342.lastWidgetIdx = var347.lastWidgetIdx;
		var342.scriptWasLoadedAsync = var347.scriptWasLoadedAsync;
		var342.apiJsReloadBackoffMs = var348 ? num9 : pick11(var347);
		var342.apiJsReloadNextAllowedTsMs = pick12(var347);
		var342.apiJsMismatchReloadAttempts = (var344 = var347.apiJsMismatchReloadAttempts) !== null && var344 !== undefined ? var344 : 0;
		var342.apiJsMismatchReloadCompletedCount = ((var345 = var347.apiJsMismatchReloadCompletedCount) !== null && var345 !== undefined ? var345 : 0) + (var348 ? 1 : 0);
		var342.apiJsMismatchReloadPending = false;
		var342.upgradeAttempts = var347.upgradeAttempts;
		var342.upgradeCompletedCount = var347.upgradeCompletedCount + 1;
		var342.turnstileLoadInitTimeTsMs = 0;
		var342.watchCatInterval = null;
		var342.watchCatSeq = var347.watchCatSeq;
		var342.widgetMap = var347.widgetMap;
		fn38(var347, true);
		var347.msgHandler && window.removeEventListener("message", var347.msgHandler);
		fn41(var346);
		var343();
		return true;
	}
	function notEqual1(var349) {
		return [
			"auto",
			"dark",
			"light"
		].indexOf(var349) !== -1;
	}
	function notEqual2(var350) {
		return ["auto", "never"].indexOf(var350) !== -1;
	}
	function and1(var351) {
		return var351 > 0 && var351 < 9e5;
	}
	function and2(var352) {
		return var352 > 0 && var352 < 36e4;
	}
	var regex1 = /^[0-9A-Za-z_-]{3,100}$/;
	function call16(var353) {
		return regex1.test(var353);
	}
	var regex2 = /^[a-z0-9_-]{0,32}$/i;
	function pick13(var354) {
		return var354 === undefined ? true : typeof var354 == "string" && regex2.test(var354);
	}
	var regex3 = /^[a-z0-9_\-=]{0,255}$/i;
	function pick14(var355) {
		return var355 === undefined ? true : typeof var355 == "string" && regex3.test(var355);
	}
	function notEqual3(var356) {
		return [
			var62.Normal,
			var62.Compact,
			var62.Invisible,
			var62.Flexible
		].indexOf(var356) !== -1;
	}
	function notEqual4(var357) {
		return [
			"auto",
			"manual",
			"never"
		].indexOf(var357) !== -1;
	}
	function notEqual5(var358) {
		return [
			"auto",
			"manual",
			"never"
		].indexOf(var358) !== -1;
	}
	var regex4 = /^[a-z]{2,3}([-_][a-z]{2})?$/i;
	function or2(var359) {
		return var359 === "auto" || regex4.test(var359);
	}
	function notEqual6(var360) {
		return [
			"always",
			"execute",
			"interaction-only"
		].indexOf(var360) !== -1;
	}
	function notEqual7(var361) {
		return ["true", "false"].indexOf(var361) !== -1;
	}
	function notEqual8(var362) {
		return ["render", "execute"].indexOf(var362) !== -1;
	}
	function fn44(var363, var364) {
		if (var363.isResetting = false, var364) {
			var364(String(num5));
			return;
		}
		fn23("Could not load challenge from challenges.cloudflare.com.", 161);
	}
	var var365 = Symbol(), str14 = "Turnstile has already been rendered in this container. The render attempt was rejected.", str15 = "Turnstile skipped implicit render because a widget already exists in this container.", win = window;
	function and3(var366) {
		return (typeof var366 == "undefined" ? "undefined" : typeOf(var366)) === "object" && var366 !== null && "clearPendingApiJsReloadRequest" in var366 && typeof var366.clearPendingApiJsReloadRequest == "function" && "rejectPendingApiJsReloadRequest" in var366 && typeof var366.rejectPendingApiJsReloadRequest == "function" && "rearmTimedUpgrade" in var366 && typeof var366.rearmTimedUpgrade == "function" && "reloadAfterUpgrade" in var366 && typeof var366.reloadAfterUpgrade == "function";
	}
	function fn45(var367) {
		var var368;
		if (!((typeof var367 == "undefined" ? "undefined" : typeOf(var367)) !== "object" || var367 === null)) {
			var var369 = (var368 = Object.getOwnPropertyDescriptor(var367, var365)) === null || var368 === undefined ? undefined : var368.value;
			if (and3(var369)) return var369;
			var var370 = fn25(var367, and3);
			if (var370) return var370;
		}
	}
	var obj9 = {
		apiJsMismatchReloadAttempts: 0,
		apiJsMismatchReloadCompletedCount: 0,
		apiJsMismatchReloadPending: false,
		apiJsReloadBackoffMs: 3e4,
		apiJsReloadNextAllowedTsMs: 0,
		apiVersion: 1,
		isReady: false,
		isRecaptchaCompatibilityMode: false,
		lastWidgetIdx: 0,
		scriptUrl: "undefined",
		scriptUrlParsed: undefined,
		scriptWasLoadedAsync: false,
		turnstileLoadInitTimeTsMs: 0,
		upgradeAttempts: 0,
		upgradeCompletedCount: 0,
		watchCatInterval: null,
		watchCatSeq: 0,
		widgetMap: new Map()
	}, var371, var372, var373;
	function fn46(var374) {
		var flag14 = true, flag15 = false, var375 = undefined;
		try {
			for (var var376 = obj9.widgetMap[Symbol.iterator](), var377; !(flag14 = (var377 = var376.next()).done); flag14 = true) {
				var var378 = or1(var377.value, 2), var379 = var378[0], var380 = var378[1];
				if (var380.wrapper.parentElement === var374 || var380.wrapper !== var374 && var380.wrapper.contains(var374) || var380.shadow.contains(var374)) return var379;
			}
		} catch (var381) {
			flag15 = true;
			var375 = var381;
		} finally {
			try {
				!flag14 && var376.return != null && var376.return();
			} finally {
				if (flag15) throw var375;
			}
		}
		return null;
	}
	function call17(var382) {
		if (typeof var382 == "string") {
			var var383 = pick3(var382);
			return var383 && obj9.widgetMap.has(var383) ? var383 : obj9.widgetMap.has(var382) ? var382 : null;
		}
		return fn46(var382);
	}
	function pick15(var384) {
		return var384 === "implicit" ? str15 : str14;
	}
	function and4(var385, var386) {
		return var385 === "explicit" && (var386 == null ? undefined : var386.renderSource) === "implicit";
	}
	function fn47() {
		var371 !== undefined && (window.clearTimeout(var371), var371 = undefined);
	}
	function fn48(var387) {
		var var388 = obj9.widgetMap.get(var387), var389 = call8(var387);
		if (!(!var388 || !var389)) {
			var elem7 = var388.shadow.querySelector("#".concat(var389));
			elem7 && fn30(elem7, {
				apiJsMismatchReloadAttempts: obj9.apiJsMismatchReloadAttempts,
				apiJsMismatchReloadCompletedCount: obj9.apiJsMismatchReloadCompletedCount,
				event: "reloadApiJsRejected",
				source: str1,
				widgetId: var387
			}, var388.iframeOrigin);
		}
	}
	function fn49() {
		var var390 = (arguments.length > 0 && arguments[0] !== undefined ? arguments[0] : {}).preserveMismatchReloadPending, var391 = var390 === undefined ? false : var390;
		var373 = undefined;
		var391 || (obj9.apiJsMismatchReloadPending = false);
		var372 !== undefined && (window.clearTimeout(var372), var372 = undefined);
	}
	function fn50() {
		var var392 = var373;
		fn49();
		var392 !== undefined && fn48(var392);
	}
	function fn51() {
		var var393 = obj9.apiJsReloadBackoffMs, var394 = Number.isFinite(var393) && var393 > 0 ? Math.min(var393, num10) : num9;
		obj9.apiJsReloadNextAllowedTsMs = 0 + Math.round(var394 * (.8 + Math.random() * .4));
		obj9.apiJsReloadBackoffMs = Math.min(var394 * 2, num10);
	}
	function fn52(var396) {
		fn49({ preserveMismatchReloadPending: true });
		obj9.apiJsMismatchReloadPending = true;
		var373 = var396;
		var372 = window.setTimeout(function() {
			fn50();
		}, num11);
	}
	function fn53() {
		fn47();
		true && (var371 = window.setTimeout(function() {
			var371 = undefined;
			pick22();
		}, num13));
	}
	function fn54(var397, var398) {
		fn58(var397, var398, "");
	}
	var arr7 = [];
	function fn55() {
		obj9.isReady = true;
		var flag16 = true, flag17 = false, var399 = undefined;
		try {
			for (var var400 = arr7[Symbol.iterator](), var401; !(flag16 = (var401 = var400.next()).done); flag16 = true) {
				var var402 = var401.value;
				var402();
			}
		} catch (var403) {
			flag17 = true;
			var399 = var403;
		} finally {
			try {
				!flag16 && var400.return != null && var400.return();
			} finally {
				if (flag17) throw var399;
			}
		}
	}
	function fn56(var404, var405) {
		var404.onerror = function() {
			fn44(var405, var405.cbError);
		};
	}
	function fn57(var406, var407) {
		var var408, var409, var410 = (var408 = var406.params["response-field"]) !== null && var408 !== undefined ? var408 : true, var411 = var410 && typeof var406.params["response-field-name"] != "string" && or3((var409 = var406.params.sitekey) !== null && var409 !== undefined ? var409 : ""), var412 = obj9.isRecaptchaCompatibilityMode, var413 = "".concat(var407, "_response"), var414 = "".concat(var407, "_legacy_response"), var415 = "".concat(var407, "_g_response"), var416 = (!var410 || instanceOf(document.getElementById(var413), HTMLInputElement)) && (!var411 || instanceOf(document.getElementById(var414), HTMLInputElement)) && (!var412 || instanceOf(document.getElementById(var415), HTMLInputElement));
		if (!(var406.responseElementsBuilt && var416)) {
			if (var410 && !instanceOf(document.getElementById(var413), HTMLInputElement)) {
				var var417, elem8 = document.createElement("input");
				elem8.type = "hidden";
				elem8.name = (var417 = var406.params["response-field-name"]) !== null && var417 !== undefined ? var417 : str6;
				elem8.id = var413;
				var406.wrapper.appendChild(elem8);
			}
			if (var411 && !instanceOf(document.getElementById(var414), HTMLInputElement)) {
				var elem9 = document.createElement("input");
				elem9.type = "hidden";
				elem9.name = str5;
				elem9.id = var414;
				var406.wrapper.appendChild(elem9);
			}
			if (var412 && !instanceOf(document.getElementById(var415), HTMLInputElement)) {
				var elem10 = document.createElement("input");
				elem10.type = "hidden";
				elem10.name = str7;
				elem10.id = var415;
				var406.wrapper.appendChild(elem10);
			}
			var406.responseElementsBuilt = true;
		}
	}
	function fn58(var418, var419, var420) {
		fn57(var418, var419);
		var elem11 = document.getElementById("".concat(var419, "_response"));
		elem11 !== null && instanceOf(elem11, HTMLInputElement) && (elem11.value = var420);
		var elem12 = document.getElementById("".concat(var419, "_legacy_response"));
		if (elem12 !== null && instanceOf(elem12, HTMLInputElement) && (elem12.value = var420), obj9.isRecaptchaCompatibilityMode) {
			var elem13 = document.getElementById("".concat(var419, "_g_response"));
			elem13 !== null && instanceOf(elem13, HTMLInputElement) && (elem13.value = var420);
		}
	}
	function negate(var421, var422) {
		var var423;
		return !!(!((var423 = var422.kills) === null || var423 === undefined) && var423.includes(var421));
	}
	function fn59(var424, var425) {
		var var426 = var425.params.size, var427 = var426 === undefined ? "normal" : var426, var428 = var425.mode;
		switch (var428) {
			case var60.NonInteractive:
			case var60.Managed:
				switch (var424.style.opacity = "", var424.style.position = "", var424.style.left = "", var424.style.top = "", var424.style.visibility = "", var424.style.pointerEvents = "", var424.style.zIndex = "", var427) {
					case var62.Compact:
						var424.style.width = "150px";
						var424.style.height = "140px";
						break;
					case var62.Invisible: fn23("Invalid value for parameter \"size\", expected \"".concat(var62.Compact, "\", \"").concat(var62.Flexible, "\", or \"").concat(var62.Normal, "\", got \"").concat(var427, "\""), 2817);
					case var62.Normal:
						var424.style.width = "300px";
						var424.style.height = "65px";
						break;
					case var62.Flexible:
						var424.style.width = "100%";
						var424.style.maxWidth = "100vw";
						var424.style.minWidth = "300px";
						var424.style.height = "65px";
						break;
					default: break;
				}
				break;
			case var60.Invisible:
				negate("floating-invisible", var425) ? (var424.style.width = "0", var424.style.height = "0", var424.style.position = "absolute", var424.style.visibility = "hidden") : (var424.style.width = "1px", var424.style.height = "1px", var424.style.opacity = "0.01", var424.style.position = "fixed", var424.style.left = "0", var424.style.top = "0", var424.style.visibility = "visible", var424.style.pointerEvents = "none", var424.style.zIndex = "-1");
				var424.setAttribute("tabindex", "-1");
				var424.setAttribute("aria-hidden", "true");
				break;
			default: fn23("Invalid value for parameter \"mode\", expected \"".concat(var60.NonInteractive, "\", \"").concat(var60.Managed, "\" or \"").concat(var60.Invisible, "\", got \"").concat(var428, "\""), 2818);
		}
	}
	function fn60(var429) {
		var429.style.width = "1px";
		var429.style.height = "1px";
		var429.style.opacity = "0.01";
		var429.style.position = "fixed";
		var429.style.left = "0";
		var429.style.top = "0";
		var429.style.visibility = "visible";
		var429.style.pointerEvents = "none";
		var429.style.zIndex = "-1";
	}
	function fn61(var430, var431) {
		var var432 = var431.get("turnstile_iframe_alt");
		var432 && (var430.title = var432);
	}
	function pick16(var433, var434) {
		var var435, var436;
		return (var435 = (var436 = var433.wrapper.parentNode) === null || var436 === undefined ? undefined : var436.querySelector("#".concat(var434, "-fr"))) !== null && var435 !== undefined ? var435 : null;
	}
	function pick17(var437) {
		var var438, var439;
		return !((var439 = var437.feedbackPopup) === null || var439 === undefined) && var439.closed ? (var437.feedbackPopup = undefined, var437.feedbackPopupOrigin = undefined, null) : (var438 = var437.feedbackPopup) !== null && var438 !== undefined ? var438 : null;
	}
	function fn62(var440, var441) {
		var var442, var443, var444 = (var442 = (var443 = pick16(var440, var441)) === null || var443 === undefined ? undefined : var443.contentWindow) !== null && var442 !== undefined ? var442 : null;
		if (var444) return {
			targetOrigin: var440.feedbackIframeOrigin,
			targetWindow: var444
		};
		var var445 = pick17(var440);
		return {
			targetOrigin: var445 ? var440.feedbackPopupOrigin : undefined,
			targetWindow: var445
		};
	}
	function and5(var446) {
		if ((typeof var446 == "undefined" ? "undefined" : typeOf(var446)) !== "object" || var446 === null) return false;
		var var447 = var446;
		return var447.source === str1 && typeof var447.event == "string" && typeof var447.widgetId == "string";
	}
	function and6(var448) {
		return var448.isTrusted && and5(var448.data);
	}
	function call18(var449) {
		return pick7(var449.origin, false);
	}
	function fn63(var450, var451, var452) {
		var var453, var454, var455 = (var453 = (var454 = var451.shadow.querySelector("#".concat(var452))) === null || var454 === undefined ? undefined : var454.contentWindow) !== null && var453 !== undefined ? var453 : null, var456 = fn62(var451, var452).targetWindow, and7 = function(var457) {
			return var457 !== null && var450.source === var457;
		};
		switch (var450.data.event) {
			case "requestFeedbackData":
			case "closeFeedbackReportIframe": return and7(var456);
			case "refreshRequest": return and7(var455) || var450.data.reason === "feedback_refresh" && and7(var456);
			default: return and7(var455);
		}
	}
	function or3(var458) {
		return var458.startsWith("0x4AAAAAAAAAA") || var458.startsWith("0x4AAAAAAAAj");
	}
	function call19() {
		return pick9(window);
	}
	function pick18(var459, var460, var461) {
		return var459 === null ? var460 : notEqual7(var459) ? var459 === "true" : (fn24(var461(var459)), var460);
	}
	function fn64() {
		try {
			var var462 = fn26();
			if (!var462) return;
			var var463 = var462.src, flag18 = true, flag19 = false, var464 = undefined;
			try {
				for (var var465 = performance.getEntriesByType("resource")[Symbol.iterator](), var466; !(flag18 = (var466 = var465.next()).done); flag18 = true) {
					var var467 = var466.value;
					if (instanceOf(var467, PerformanceResourceTiming) && var467.name.includes(var463)) return var467;
				}
			} catch (var468) {
				flag19 = true;
				var464 = var468;
			} finally {
				try {
					!flag18 && var465.return != null && var465.return();
				} finally {
					if (flag19) throw var464;
				}
			}
		} catch (var469) {}
	}
	var var470 = (function() {
		var call20 = function(var471, var472, var473, var474, var475, var476) {
			return fn2(function() {
				var var477, var478, var479, var480, var481, var482, var483, var484, var485, var486;
				return fn14(this, function(var487) {
					switch (var487.label) {
						case 0:
							if (var477 = function(var488, var489) {
								var var490 = obj9.widgetMap.get(var472);
								var490 !== var471 || var490.isComplete || var490.isResetting || var490.response !== var474 || (!var488 && var489 && fn24(var489), var499(var490, var473, var488));
							}, var478 = var471.params.sitekey, var479 = call19(), !var479) return fn24("Cannot determine Turnstile's embedded location, aborting clearance redemption."), var499(var471, var473, false), [2];
							var480 = "h/".concat("b", "/");
							var481 = new URL(var479);
							var482 = "https";
							var483 = "";
							var484 = "".concat(var482, "://").concat(var481.host, "/cdn-cgi/challenge-platform/").concat(var480, "rc/").concat(var476).concat(var483);
							var487.label = 1;
						case 1:
							var487.trys.push([
								1,
								3,
								,
								4
							]);
							return [4, fetch(var484, {
								body: JSON.stringify({
									secondaryToken: var475,
									sitekey: var478
								}),
								headers: { "Content-Type": "application/json" },
								method: "POST",
								redirect: "manual"
							})];
						case 2:
							var485 = var487.sent();
							var485.status === 200 ? var477(true) : var477(false, "Cannot determine Turnstile's embedded location, aborting clearance redemption, are you running Turnstile on a Cloudflare Zone?");
							return [3, 4];
						case 3:
							var486 = var487.sent();
							var477(false, "Error contacting Turnstile, aborting clearance redemption.");
							return [3, 4];
						case 4: return [2];
					}
				});
			})();
		}, var491 = function(var492, var493, var494) {
			if (var492.params.retry === var68.Auto || var494) {
				var var495;
				var492.retryTimeout = window.setTimeout(function() {
					var534(var494 ? var80.CrashedRetry : var80.FailureRetry, var493);
				}, var494 ? 0 : 2e3 + ((var495 = var492.params["retry-interval"]) !== null && var495 !== undefined ? var495 : 0));
			}
		}, pick19 = function(var496, var497, var498) {
			return var496.params.execution === var76.Render ? true : (var497 === var80.CrashedRetry || var497 === var80.FailureRetry || var497 === var80.CheckDelays || var497 === var80.UpgradeReload) && var496.params.execution === var76.Execute && var498;
		}, var499 = function(var500, var501, var502) {
			var var503;
			var500.response === undefined && fn23("[Internal Error] Widget was completed but no response was given", 1362);
			var500.isExecuting = false;
			var500.isComplete = true;
			fn58(var500, var501, var500.response);
			(var503 = var500.cbSuccess) === null || var503 === undefined || var503.call(var500, var500.response, var502);
		}, var504 = function(var505) {
			if (!var505) return [];
			for (var list6 = var505.attributes, var506 = list6.length, arr8 = new Array(var506), num27 = 0; num27 < var506; num27++) arr8[num27] = list6[num27].name;
			return arr8;
		}, var507 = function() {
			for (var obj10 = {}, arr9 = [], elems1 = document.getElementsByTagName("*"), num28 = 0; num28 < elems1.length && arr9.length < 50; num28++) {
				var list7 = elems1[num28].tagName.toLowerCase();
				list7.indexOf("-") !== -1 && !obj10[list7] && (obj10[list7] = true, arr9.push(list7));
			}
			return arr9;
		}, var508 = function(var509, var510, var511) {
			if (var509.rcV = var510, 0) var var512;
		}, var513 = function() {
			var str16 = "";
			do {
				str16 = "";
				for (var num29 = 0; num29 < 5; num29++) str16 += "abcdefghijklmnopqrstuvwxyz0123456789".charAt(Math.floor(Math.random() * 36));
			} while (obj9.widgetMap.has(str16));
			return str16;
		}, var515 = function(var516, var517, var518) {
			for (; var516.msgQueue.length > 0;) {
				fn30(var518, {
					event: var516.msgQueue.pop(),
					source: str1,
					widgetId: var517
				}, var516.iframeOrigin);
			}
		}, var519 = function(var520, var521) {
			if (var521) {
				var arr10 = [
					"retry-interval",
					"retry",
					"size",
					"theme",
					"tabindex",
					"execution",
					"refresh-expired",
					"refresh-timeout",
					"response-field-name",
					"response-field",
					"language",
					"base-url",
					"appearance",
					"sitekey",
					"feedback-enabled"
				], var522 = Object.prototype.hasOwnProperty, arr11 = [], flag20 = true, flag21 = false, var523 = undefined;
				try {
					for (var var524 = arr10[Symbol.iterator](), var525; !(flag20 = (var525 = var524.next()).done); flag20 = true) {
						var var526 = var525.value;
						var522.call(var521, var526) && var521[var526] !== undefined && var521[var526] !== var520.params[var526] && arr11.push(var526);
					}
				} catch (var527) {
					flag21 = true;
					var523 = var527;
				} finally {
					try {
						!flag20 && var524.return != null && var524.return();
					} finally {
						if (flag21) throw var523;
					}
				}
				arr11.length > 0 && fn23("The parameters ".concat(arr10.join(","), " is/are not allowed be changed between the calls of render() and execute() of a widget.\n    Consider rendering a new widget if you want to change the following parameters ").concat(arr11.join(",")), 3618);
				var521.action && (pick13(var521.action) || fn23("Invalid input for optional parameter \"action\", got \"".concat(var521.action, "\""), 3604), var520.action = var521.action);
				var521.cData && (pick14(var521.cData) || fn23("Invalid input for optional parameter \"cData\", got \"".concat(var521.cData, "\""), 3605), var520.cData = var521.cData);
				var521["after-interactive-callback"] && (var520.cbAfterInteractive = var521["after-interactive-callback"]);
				var521["before-interactive-callback"] && (var520.cbBeforeInteractive = var521["before-interactive-callback"]);
				var521.callback && (var520.cbSuccess = var521.callback);
				var521["expired-callback"] && (var520.cbExpired = var521["expired-callback"]);
				var521["timeout-callback"] && (var520.cbTimeout = var521["timeout-callback"]);
				var521["error-callback"] && (var520.cbError = var521["error-callback"]);
				var521["unsupported-callback"] && (var520.cbUnsupported = var521["unsupported-callback"]);
				var521.chlPageData && (var520.chlPageData = var521.chlPageData);
			}
		}, var528 = function(var529, var530, var531) {
			var529 === "explicit" && var530 && var519(var530, var531);
			fn24(pick15(var529));
		}, var532 = function(var533) {
			var534(var80.Api, var533);
		}, var534 = function(var535, var536) {
			var var537 = pick20(var536);
			var537 || fn23("Nothing to reset found for provided container", 3329);
			var var538 = obj9.widgetMap.get(var537);
			if (var538) {
				var var539, var540, var541 = var538.isExecuted;
				var538.isResetting = true;
				var538.response = undefined;
				var538.mode = undefined;
				var538.msgQueue = [];
				var538.isComplete = false;
				var538.isExecuted = false;
				var538.isExecuting = false;
				var538.isExpired = false;
				var538.isFailed = false;
				var538.isInitialized = false;
				var538.isStale = false;
				var538.isOverrunning = false;
				var538.cfChlOut = undefined;
				var538.cfChlOutS = undefined;
				var538.errorCode = undefined;
				var538.frMd = undefined;
				var538.autoFeedbackSent = false;
				var538.watchcat.overrunBeginSeq = 0;
				var538.watchcat.lastAckedSeq = 0;
				var538.watchcat.seq = 0;
				pick19(var538, var535, var541) && (var538.msgQueue.push(var78.Execute), var538.isExecuted = true, var538.isExecuting = true);
				var var542 = call8(var537), elem14 = var538.shadow.querySelector("#".concat(var542));
				(!var542 || !elem14) && fn23("Widget ".concat(var537, " to reset was not found."), 3330);
				(var538.params.appearance === var74.InteractionOnly || var538.params.appearance === var74.Execute) && fn60(elem14);
				var538.params.sitekey === null && fn23("Unexpected Error: Sitekey is null", 3347);
				var var543 = elem14.cloneNode(), var544 = call(var537, var538.params.sitekey, var538.params, (var539 = var538.rcV) !== null && var539 !== undefined ? var539 : str9, false, "b", var535, obj9.scriptUrlParsed, fn17(var538));
				var543.src = var544;
				fn56(var543, var538);
				var538.iframeOrigin = fn28(var544);
				(var540 = elem14.parentNode) === null || var540 === undefined || var540.replaceChild(var543, elem14);
				fn54(var538, var542);
				var538.retryTimeout && window.clearTimeout(var538.retryTimeout);
			} else fn23("Widget ".concat(var537, " to reset was not found."), 3331);
		}, var545 = function(var546, var547) {
			var var548 = call8(var546);
			if (var548) {
				var arr12 = [
					"input#".concat(var548, "_response"),
					"input#".concat(var548, "_legacy_response"),
					"input#".concat(var548, "_g_response")
				];
				document.querySelectorAll(arr12.join(", ")).forEach(function(var549) {
					return var549.remove();
				});
				var547.shadow.querySelectorAll(arr12.join(", ")).forEach(function(var550) {
					return var550.remove();
				});
				var246(var548);
				var200(var547);
			}
			var547.wrapper.remove();
			var547.retryTimeout && window.clearTimeout(var547.retryTimeout);
			obj9.widgetMap.delete(var546);
			fn38(obj9);
		}, var551 = function(var552) {
			var var553 = pick20(var552), var554 = var553 ? obj9.widgetMap.get(var553) : undefined;
			if (!var553 || !var554) {
				fn24("Nothing to remove found for the provided container.");
				return;
			}
			var545(var553, var554);
		}, var555 = function() {
			var var556 = Array.from(obj9.widgetMap.keys()), flag22 = true, flag23 = false, var557 = undefined;
			try {
				for (var var558 = var556[Symbol.iterator](), var559; !(flag22 = (var559 = var558.next()).done); flag22 = true) {
					var var560 = var559.value, var561 = obj9.widgetMap.get(var560), var562 = call8(var560);
					!var562 || !var561 || (var246(var562), var200(var561), var534(var80.UpgradeReload, var562));
				}
			} catch (var563) {
				flag23 = true;
				var557 = var563;
			} finally {
				try {
					!flag22 && var558.return != null && var558.return();
				} finally {
					if (flag23) throw var557;
				}
			}
		}, var564 = function(var565, var566, var567) {
			var var568, var569, var570, var571, var572, var573, var574, var575, var576, var577, var578, var579, var580, var581, var582;
			if (typeof var565 == "string") {
				var var583 = call17(var565);
				if (var583) {
					var var584, var585 = obj9.widgetMap.get(var583), var586 = (var584 = var585 == null ? undefined : var585.wrapper.parentElement) !== null && var584 !== undefined ? var584 : null;
					if (var586 && and4(var567, var585)) {
						var581 = var586;
						var582 = {
							widget: var585,
							widgetId: var583
						};
					} else return var528(var567, var585, var566), call8(var583);
				} else {
					var var587;
					try {
						var587 = document.querySelector(var565);
					} catch (var588) {
						fn23("Invalid type for \"container\", expected \"selector\" or an implementation of \"HTMLElement\", got \"".concat(var565, "\""), 3586);
					}
					var587 || fn23("Unable to find a container for \"".concat(var565, "\""), 3585);
					var581 = var587;
				}
			} else instanceOf(var565, HTMLElement) ? var581 = var565 : fn23("Invalid type for parameter \"container\", expected \"string\" or an implementation of \"HTMLElement\"", 3587);
			if (!var582) {
				var var589 = call17(var581);
				if (var589) {
					var var590 = obj9.widgetMap.get(var589);
					if (var590 && and4(var567, var590) && var590.wrapper.parentElement === var581) var582 = {
						widget: var590,
						widgetId: var589
					};
					else return var528(var567, var590, var566), call8(var589);
				}
			}
			var var591 = call19();
			if (!var591) return fn23("Turnstile cannot determine its page location", 3607);
			var var592 = fn65(var581);
			if (var592) {
				var var593 = Object.assign(var592, var566), var594 = var593.action, var595 = var593.cData, var596 = var593.chlPageData, var597 = var593.sitekey;
				var593.theme = (var568 = var593.theme) !== null && var568 !== undefined ? var568 : var64.Auto;
				var593.retry = (var569 = var593.retry) !== null && var569 !== undefined ? var569 : var68.Auto;
				var593.execution = (var570 = var593.execution) !== null && var570 !== undefined ? var570 : var76.Render;
				var593.appearance = (var571 = var593.appearance) !== null && var571 !== undefined ? var571 : var74.Always;
				var593["retry-interval"] = Number((var572 = var593["retry-interval"]) !== null && var572 !== undefined ? var572 : num12);
				var593["expiry-interval"] = Number((var573 = var593["expiry-interval"]) !== null && var573 !== undefined ? var573 : (num2 - num3) * 1e3);
				var593.size = (var574 = var593.size) !== null && var574 !== undefined ? var574 : var62.Normal;
				var var598 = var593.callback, var599 = var593["expired-callback"], var600 = var593["timeout-callback"], var601 = var593["after-interactive-callback"], var602 = var593["before-interactive-callback"], var603 = var593["error-callback"], var604 = var593["unsupported-callback"];
				typeof var597 != "string" && fn23("Invalid or missing type for parameter \"sitekey\", expected \"string\", got \"".concat(typeof var597 == "undefined" ? "undefined" : typeOf(var597), "\""), 3588);
				call16(var597) || fn23("Invalid input for parameter \"sitekey\", got \"".concat(var597, "\""), 3589);
				notEqual3(var593.size) || fn23("Invalid type for parameter \"size\", expected normal|compact, got \"".concat(var593.size, "\" ").concat(typeOf(var593.size)), 3590);
				notEqual1(var593.theme) || fn23("Invalid type for parameter \"theme\", expected dark|light|auto, got \"".concat(var593.theme, "\" ").concat(typeOf(var593.theme)), 3591);
				notEqual2(var593.retry) || fn23("Invalid type for parameter \"retry\", expected never|auto, got \"".concat(var593.retry, "\" ").concat(typeOf(var593.retry)), 3592);
				var593.language || (var593.language = "auto");
				or2(var593.language) || (fn24("Invalid language value: \"".concat(var593.language, ", expected either: auto, or an ISO 639-1 two-letter language code (e.g. en) or language and country code (e.g. en-US).")), var593.language = "auto");
				notEqual6(var593.appearance) || fn23("Unknown appearance value: \"".concat(var593.appearance, ", expected either: 'always', 'execute', or 'interaction-only'."), 3600);
				notEqual8(var593.execution) || fn23("Unknown execution value: \"".concat(var593.execution, ", expected either: 'render' or 'execute'."), 3601);
				and1(var593["retry-interval"]) || fn23("Invalid retry-interval value: \"".concat(var593["retry-interval"], ", expected an integer value > 0 and < 900000\""), 3602);
				and2(var593["expiry-interval"]) || fn23("Invalid expiry-interval value: \"".concat(var593["expiry-interval"], ", expected an integer value > 0 and < 360000\""), 3602);
				var var605 = (var575 = var593["refresh-expired"]) !== null && var575 !== undefined ? var575 : var70.Auto;
				notEqual4(var605) ? var593["refresh-expired"] = var605 : fn23("Invalid type for parameter \"refresh-expired\", expected never|manual|auto, got \"".concat(var605, "\" ").concat(typeof var605 == "undefined" ? "undefined" : typeOf(var605)), 3603);
				var var606 = (var576 = var593["refresh-timeout"]) !== null && var576 !== undefined ? var576 : var72.Auto;
				notEqual5(var606) ? var593["refresh-timeout"] = var606 : fn23("Invalid type for parameter \"refresh-timeout\", expected never|manual|auto, got \"".concat(var606, "\" ").concat(typeof var606 == "undefined" ? "undefined" : typeOf(var606)), 3603);
				pick13(var594) || fn23("Invalid input for optional parameter \"action\", got \"".concat(var594, "\""), 3604);
				pick14(var595) || fn23("Invalid input for optional parameter \"cData\", got \"".concat(var595, "\""), 3605);
				var elem15 = document.createElement("iframe"), elem16 = document.createElement("div"), var607 = elem16.attachShadow({ mode: "closed" }), var608 = var513(), var609 = call8(var608);
				if (!(!var608 || !var609)) {
					var arr13 = [], var610 = var593.execution === var76.Render;
					var610 && arr13.push(var78.Execute);
					obj9.lastWidgetIdx++;
					var obj11 = {};
					obj9.widgetMap.set(var608, fn8(fn6({
						action: var594,
						assetCtxCallback: var593._acCb,
						autoFeedbackSent: false,
						cbAfterInteractive: var601,
						cbBeforeInteractive: var602,
						cbError: var603,
						cbExpired: var599,
						cbSuccess: var598,
						cbTimeout: var600,
						cbUnsupported: var604,
						cData: var595,
						chlPageData: var596,
						idx: obj9.lastWidgetIdx,
						isComplete: false,
						isExecuted: var610,
						isExecuting: var610,
						isExpired: false,
						isFailed: false,
						isInitialized: false,
						isResetting: false,
						isStale: false,
						msgQueue: arr13,
						params: var593,
						rcV: str9,
						renderSource: var567,
						responseElementsBuilt: false,
						watchcat: {
							lastAckedSeq: 0,
							missingWidgetWarning: false,
							overrunBeginSeq: 0,
							seq: 0
						}
					}, obj11), {
						isOverrunning: false,
						shadow: var607,
						widgetInitStartTimeTsMs: 0,
						widgetParamsStartTimeTsMs: 0,
						widgetRenderEndTimeTsMs: 0,
						widgetRenderStartTimeTsMs: 0,
						wrapper: elem16
					}));
					fn37(obj9);
					var var611 = obj9.widgetMap.get(var608);
					var611 || fn23("Turnstile Initialization Error ", 3606);
					var611.chlPageData && fn47();
					elem15.style.display = "none";
					elem15.style.border = "none";
					elem15.style.overflow = "hidden";
					var var612 = call(var608, var597, var593, str9, false, "b", var80.New, obj9.scriptUrlParsed, fn17(var611));
					var611.iframeOrigin = fn28(var612);
					elem15.setAttribute("src", var612);
					fn56(elem15, var611);
					var arr14 = [
						"cross-origin-isolated",
						"fullscreen",
						"autoplay",
						"keyboard-map",
						"gamepad",
						"xr-spatial-tracking"
					];
					((var577 = (var580 = document.featurePolicy) === null || var580 === undefined || (var579 = var580.features) === null || var579 === undefined ? undefined : var579.call(var580)) !== null && var577 !== undefined ? var577 : []).indexOf(str8) !== -1 && arr14.push(str8);
					elem15.setAttribute("allow", arr14.join("; "));
					elem15.setAttribute("sandbox", "allow-same-origin allow-scripts allow-popups");
					elem15.id = var609;
					elem15.tabIndex = (var578 = var593.tabindex) !== null && var578 !== undefined ? var578 : 0;
					elem15.title = "Widget containing a Cloudflare security challenge";
					var607.appendChild(elem15);
					fn57(var611, var609);
					var582 && var545(var582.widgetId, var582.widget);
					var581.appendChild(elem16);
					var611.widgetRenderEndTimeTsMs = 0;
					return var609;
				}
			}
		}, call21 = function(var613, var614) {
			return var564(var613, var614, "explicit");
		}, var615 = function() {
			var arr15 = [str2, str3];
			obj9.isRecaptchaCompatibilityMode && arr15.push(str4);
			document.querySelectorAll(arr15.join(", ")).forEach(function(var616) {
				var564(var616, undefined, "implicit");
			});
		}, var617 = function() {
			var var618, var619 = -1, flag24 = true, flag25 = false, var620 = undefined;
			try {
				for (var var621 = obj9.widgetMap[Symbol.iterator](), var622; !(flag24 = (var622 = var621.next()).done); flag24 = true) {
					var var623 = or1(var622.value, 2), var624 = var623[0], var625 = var623[1];
					var619 < var625.idx && (var618 = var624, var619 = var625.idx);
				}
			} catch (var626) {
				flag25 = true;
				var620 = var626;
			} finally {
				try {
					!flag24 && var621.return != null && var621.return();
				} finally {
					if (flag25) throw var620;
				}
			}
			var619 === -1 && fn23("Could not find widget", 43778);
			return var618;
		}, var627 = fn64(), var628 = function(var629) {
			if (var629.source === str1 && !(!var629.widgetId || !obj9.widgetMap.has(var629.widgetId))) {
				var var630 = call8(var629.widgetId), var631 = obj9.widgetMap.get(var629.widgetId);
				if (!(!var630 || !var631)) switch (var629.event) {
					case "init": {
						var631.widgetInitStartTimeTsMs = 0;
						var631.kills = var629.kills;
						var elem17 = var631.shadow.getElementById(var630);
						elem17 || fn23("Cannot initialize Widget, Element not found (#".concat(var630, ")."), 3074);
						var631.mode = var629.mode;
						var631.nextRcV = var629.nextRcV;
						var631.mode === var60.Invisible && var631.params["refresh-expired"] === var70.Manual && fn24("refresh-expired=manual is impossible in invisible mode, consider using '".concat(var70.Auto, "' or '").concat(var70.Never, ".'"));
						var631.mode !== var60.Managed && var631.params["refresh-timeout"] !== var72.Auto && fn24("setting refresh-timeout has no effect on an invisible/non-interactive widget and will be ignored.");
						var631.params.appearance === var74.Always || var631.isExecuting && var631.params.appearance === var74.Execute ? fn59(elem17, var631) : fn60(elem17);
						elem17.style.display = "";
						var elem18 = var631.shadow.querySelector("#".concat(var630));
						elem18 || fn23("Received state for an unknown widget: ".concat(var629.widgetId), 3078);
						fn30(elem18, {
							event: "init",
							source: str1,
							widgetId: var629.widgetId
						}, var631.iframeOrigin);
						break;
					}
					case "translationInit": {
						var elem19 = var631.shadow.getElementById(var630);
						elem19 || fn23("Cannot initialize Widget, Element not found (#".concat(var630, ")."), 3074);
						var map = new Map();
						var631.displayLanguage = var629.displayLanguage;
						var631.displayRtl = var629.displayRtl;
						Object.keys(var629.translationData).forEach(function(var632) {
							map.set(var632, var629.translationData[var632]);
						});
						fn61(elem19, map);
						break;
					}
					case "languageUnsupported": {
						fn24("Language ".concat(var631.params.language, " is not supported, falling back to: ").concat(var629.fallback, "."));
						var631.displayLanguage = var629.fallback;
						break;
					}
					case "reject": {
						var elem20 = var631.shadow.getElementById(var630);
						if (var631.isExecuting = false, elem20 || fn23("Cannot initialize Widget, Element not found (#".concat(var630, ")."), 3075), var629.reason === "unsupported_browser") {
							var var633;
							(var633 = var631.cbUnsupported) === null || var633 === undefined || var633.call(var631);
						}
						break;
					}
					case "food": {
						var631.watchcat && var629.seq > var631.watchcat.lastAckedSeq && (var631.watchcat.lastAckedSeq = var629.seq);
						break;
					}
					case "overrunBegin": {
						var631.isOverrunning = true;
						var631.watchcat && (var631.watchcat.overrunBeginSeq = var631.watchcat.lastAckedSeq);
						break;
					}
					case "overrunEnd": {
						var631.isOverrunning = false;
						break;
					}
					case "complete": {
						if (var508(var631, str9, var629.widgetId), var631.response = var629.token, var629.aC) {
							var var634;
							(var634 = var631.assetCtxCallback) === null || var634 === undefined || var634.call(var631, var629.aC);
						}
						if (var629.scs && !negate("scs", var631) && (var631.scs = var629.scs, var631.params["session-continuity-persist"] && !negate("scs_persist", var631))) {
							var var635 = var631.params.sitekey;
							if (var635) {
								var var636 = "".concat(str10).concat(var635);
								try {
									localStorage.setItem(var636, var629.scs);
								} catch (var637) {}
							}
						}
						var629.sToken ? call20(var631, var629.widgetId, var630, var629.token, var629.sToken, var629.chlId) : var499(var631, var630, false);
						break;
					}
					case "fail": {
						if (var629.rcV && var508(var631, var629.rcV, var629.widgetId), var629.cfChlOut && (var631.cfChlOut = var629.cfChlOut), var629.cfChlOutS && (var631.cfChlOutS = var629.cfChlOutS), var629.code && (var631.errorCode = var629.code), var629.aC) {
							var var638;
							(var638 = var631.assetCtxCallback) === null || var638 === undefined || var638.call(var631, var629.aC);
						}
						var631.isExecuting = false;
						var631.isFailed = true;
						var631.isInitialized = true;
						var629.frMd && (var631.frMd = var629.frMd);
						fn54(var631, var630);
						var var639 = var631.cbError, var640 = var629.code === num7 || var629.code === num8, var641 = var629.code !== num4;
						if (var640) {
							var elem21 = var631.shadow.querySelector("#".concat(var630));
							elem21 && fn30(elem21, {
								event: "forceFail",
								source: str1,
								widgetId: var629.widgetId
							}, var631.iframeOrigin);
						}
						if (var639) {
							var var642;
							var639(String((var642 = var629.code) !== null && var642 !== undefined ? var642 : num6)) ? var641 && var631.params.retry === var68.Auto && !var631.isResetting && var491(var631, var630, var640) : (var629.code && fn24("Error: ".concat(var629.code, ".")), var641 && var491(var631, var630, var640));
						} else var629.code ? (var641 && var491(var631, var630, var640), fn23("Error: ".concat(var629.code), 3076)) : var491(var631, var630, false);
						break;
					}
					case "feedbackInit": {
						var var643 = fn62(var631, var630).targetWindow;
						if (var643) {
							fn24("A feedback report form is already opened for this widget.");
							return;
						}
						if (!var631.autoFeedbackSent && !negate("feedback-report-auto-submit", var631)) {
							var var644 = var181(var631, var629.feedbackOrigin, var629.rayId, var631.frMd, obj9.scriptUrlParsed);
							var644 && (var631.autoFeedbackSent = true);
						}
						var224(var630, var631, var629.feedbackOrigin, obj9.scriptUrlParsed);
						break;
					}
					case "requestFeedbackData": {
						var elem22 = var631.shadow.querySelector("#".concat(var630));
						elem22 || fn23("Received state for an unknown widget: #".concat(var630, " / ").concat(var629.widgetId), 3078);
						fn30(elem22, {
							event: "requestTurnstileResults",
							source: str1,
							widgetId: var629.widgetId
						}, var631.iframeOrigin);
						break;
					}
					case "turnstileResults": {
						var var645, var646, var647, var648 = fn62(var631, var630), var649 = var648.targetOrigin, var650 = var648.targetWindow;
						if (!var650) break;
						fn29(var650, {
							cfChlOut: (var645 = var631.cfChlOut) !== null && var645 !== undefined ? var645 : var629.cfChlOut,
							cfChlOutS: (var646 = var631.cfChlOutS) !== null && var646 !== undefined ? var646 : var629.cfChlOutS,
							errorCode: var631.errorCode,
							event: "feedbackData",
							frMd: (var647 = var631.frMd) !== null && var647 !== undefined ? var647 : var629.frMd,
							mode: var629.mode,
							rayId: var629.rayId,
							rcV: var629.rcV,
							sitekey: var629.sitekey,
							source: str1,
							widgetId: var629.widgetId
						}, var649);
						break;
					}
					case "closeFeedbackReportIframe": {
						var var651 = fn62(var631, var630).targetWindow;
						var651 || fn23("Received state for an unknown widget: ".concat(var629.widgetId), 3078);
						var246(var630);
						var200(var631);
						break;
					}
					case "tokenExpired": {
						var var652, var653 = var629.token;
						var631.isExpired = true;
						(var652 = var631.cbExpired) === null || var652 === undefined || var652.call(var631, var653);
						var631.params["refresh-expired"] === var70.Auto && !var631.isResetting && var534(var80.AutoExpire, var630);
						break;
					}
					case "interactiveTimeout": {
						var508(var631, str9, var629.widgetId);
						fn54(var631, var630);
						var var654 = var631.cbTimeout;
						if (var654 ? var654() : var631.params["refresh-timeout"] === var72.Never && !var631.isResetting && fn24("The widget encountered an interactive timeout and is set to never refresh. Consider defining a timeout handler and resetting the widget upon timeout as solving a widget in a timed-out state is going to fail."), var631.params["refresh-timeout"] === var72.Auto && !var631.isResetting) {
							var var655 = var631.cbAfterInteractive;
							var655 == null || var655();
							var534(var80.AutoTimeout, var630);
						}
						break;
					}
					case "refreshRequest": {
						var508(var631, str9, var629.widgetId);
						var246(var630);
						var200(var631);
						var534(var80.ManualRefresh, var630);
						break;
					}
					case "reloadRequest": {
						var508(var631, var629.nextRcV, var629.widgetId);
						var534(var629.trigger, var630);
						break;
					}
					case "reloadApiJsRequest": {
						if (negate("reload", var631)) {
							fn48(var629.widgetId);
							break;
						}
						if (var373 !== undefined) {
							fn48(var629.widgetId);
							break;
						}
						pick22() ? (obj9.apiJsMismatchReloadAttempts++, fn51(), fn52(var629.widgetId)) : fn48(var629.widgetId);
						break;
					}
					case "interactiveBegin": {
						var var656, elem23 = var631.shadow.getElementById(var630);
						elem23 || fn23("Cannot layout widget, Element not found (#".concat(var630, ")."), 3076);
						(var656 = var631.cbBeforeInteractive) === null || var656 === undefined || var656.call(var631);
						var631.params.appearance === var74.InteractionOnly && fn59(elem23, var631);
						break;
					}
					case "interactiveEnd": {
						var var657;
						(var657 = var631.cbAfterInteractive) === null || var657 === undefined || var657.call(var631);
						break;
					}
					case "widgetStale": {
						var631.isStale = true;
						break;
					}
					case "requestExtraParams": {
						var631.widgetParamsStartTimeTsMs = 0;
						var elem24 = var631.shadow.querySelector("#".concat(var630));
						elem24 || fn23("Received state for an unknown widget: ".concat(var629.widgetId), 3078);
						var631.isResetting = false;
						var obj12 = {}, num31 = 0, obj13 = {
							"d.cT": var507(),
							"ht.atrs": var504(document.body.parentNode),
							"pg.ref": document.referrer,
							pi: {
								ffp: call14(var631.wrapper),
								ii: window.self !== window.top,
								lH: window.location.href,
								mL: document.getElementsByTagName("meta").length,
								pfp: call15(document, num14, num15),
								sL: document.scripts.length,
								sR: var631.wrapper.shadowRoot === null,
								ssL: document.styleSheets.length,
								t: "".concat(document.title.length, "|").concat(shiftRightU(document.title)),
								tL: document.getElementsByTagName("*").length,
								wp: call13(var631.wrapper),
								xp: call11(var631.wrapper).slice(0, num16)
							},
							"w.iW": window.innerWidth
						}, list8 = var631.scs;
						if (!list8 && var631.params["session-continuity-persist"] && !negate("scs_persist", var631)) {
							var var658 = var631.params.sitekey;
							if (var658) {
								var var659 = "".concat(str10).concat(var658);
								try {
									var var660;
									list8 = (var660 = localStorage.getItem(var659)) !== null && var660 !== undefined ? var660 : undefined;
								} catch (var661) {}
							}
						}
						list8 && list8.length > num17 && (list8 = undefined);
						fn30(elem24, fn6({
							action: var631.action,
							apiJsMismatchReloadAttempts: obj9.apiJsMismatchReloadAttempts,
							apiJsMismatchReloadCompletedCount: obj9.apiJsMismatchReloadCompletedCount,
							apiJsResourceTiming: var627 ? JSON.parse(JSON.stringify(var627)) : undefined,
							appearance: var631.params.appearance,
							au: obj9.scriptUrl,
							cData: var631.cData,
							ch: "1eec422858ff",
							chlPageData: var631.chlPageData,
							event: "extraParams",
							execution: var631.params.execution,
							"expiry-interval": var631.params["expiry-interval"],
							language: var631.params.language,
							rcV: var631.rcV,
							"refresh-expired": var631.params["refresh-expired"],
							"refresh-timeout": var631.params["refresh-timeout"],
							retry: var631.params.retry,
							"retry-interval": var631.params["retry-interval"],
							scs: list8,
							source: str1,
							timeExtraParamsMs: 0 - var631.widgetRenderStartTimeTsMs,
							timeInitMs: var631.widgetInitStartTimeTsMs - var631.widgetRenderEndTimeTsMs,
							timeLoadInitMs: 0 - obj9.turnstileLoadInitTimeTsMs,
							timeParamsMs: var631.widgetParamsStartTimeTsMs - var631.widgetInitStartTimeTsMs,
							timeRenderMs: var631.widgetRenderEndTimeTsMs - var631.widgetRenderStartTimeTsMs,
							timeTiefMs: 0 - num31,
							upgradeAttempts: obj9.upgradeAttempts,
							upgradeCompletedCount: obj9.upgradeCompletedCount,
							url: call19(),
							widgetId: var629.widgetId,
							wPr: obj13
						}, obj12), var631.iframeOrigin);
						var515(var631, var629.widgetId, elem24);
						var631.isInitialized = true;
						break;
					}
					default: break;
				}
			}
		}, var662 = function(var663) {
			if (and6(var663)) {
				var var664 = var663.data;
				if (!call18(var663)) {
					fn24("Ignored message from wrong origin: ".concat(var663.origin, "."));
					return;
				}
				if (!(!var664.widgetId || !obj9.widgetMap.has(var664.widgetId))) {
					var var665 = call8(var664.widgetId), var666 = obj9.widgetMap.get(var664.widgetId);
					if (!(!var665 || !var666)) {
						if (!fn63(var663, var666, var665)) {
							fn24("Ignored message from unexpected source for event: ".concat(var664.event, "."));
							return;
						}
						var628(var664);
					}
				}
			}
		};
		obj9.msgHandler = var662;
		obj9.internalMsgHandler = var628;
		window.addEventListener("message", var662);
		function pick20(var667) {
			var var668;
			if (typeof var667 == "string") {
				var var669 = call17(var667);
				if (var669) return var669;
				try {
					var elem25 = document.querySelector(var667);
					return elem25 ? pick20(elem25) : null;
				} catch (var670) {
					return null;
				}
			}
			return instanceOf(var667, Element) ? call17(var667) : var667 || obj9.widgetMap.size === 0 ? null : (var668 = var617()) !== null && var668 !== undefined ? var668 : null;
		}
		var obj14 = {}, obj15 = { showFeedback: function(var671) {
			var var672 = pick20(var671);
			if (var672) {
				var var673 = call8(var672);
				if (var673) {
					var var674 = obj9.widgetMap.get(var672);
					var674 && var224(var673, var674, var253.Custom, obj9.scriptUrlParsed);
				}
			}
		} }, var675 = fn8(fn6({}, obj14), {
			_private: obj15,
			execute: function(var676, var677) {
				var var678 = pick20(var676);
				if (!var678) {
					var var679;
					var677 === undefined && fn23("Please provide 2 parameters to execute: container and parameters", 43521);
					var var680 = call21(var676, var677);
					var680 || fn23("Failed to render widget", 43522);
					var678 = (var679 = pick3(var680)) !== null && var679 !== undefined ? var679 : pick20(var676);
					var678 || fn23("Failed to render widget", 43522);
				}
				var var681 = obj9.widgetMap.get(var678);
				if (var681) {
					var519(var681, var677);
					var var682 = call8(var678);
					if (var682 || (var681.isExecuting = false, fn23("Widget ".concat(var678, " to execute was not found"), 43522)), var681.isExecuting) {
						fn24("Call to execute() on a widget that is already executing (".concat(var682, "), consider using reset() before execute()."));
						return;
					}
					if (var681.isExecuting = true, var681.response) {
						var var683;
						var681.isExecuting = false;
						fn24("Call to execute() on a widget that was already executed (".concat(var682, "), execute() will return the previous token obtained. Consider using reset() before execute() to obtain a fresh token."));
						(var683 = var681.cbSuccess) === null || var683 === undefined || var683.call(var681, var681.response, false);
						return;
					}
					var681.isExpired && fn24("Call to execute on a expired-widget (".concat(var682, "), consider using reset() before."));
					var681.isStale && (var534(var80.StaleExecute, var682), var681.isExecuting = true);
					(var681.isResetting || !var681.isInitialized) && var681.msgQueue.push(var78.Execute);
					var681.isExecuted = true;
					var elem26 = var681.shadow.querySelector("#".concat(var682));
					if (elem26 || (var681.isExecuting = false, fn23("Widget ".concat(var682, " to execute was not found"), 43522)), var681.isResetting || !var681.isInitialized) return;
					if (var681.isInitialized && var681.msgQueue.length > 0) {
						var515(var681, var678, elem26);
						var681.params.appearance === var74.Execute && fn59(elem26, var681);
						return;
					}
					var681.isInitialized && var681.params.appearance === var74.Execute && fn59(elem26, var681);
					var681.isExecuting && fn30(elem26, {
						event: "execute",
						source: str1,
						widgetId: var678
					}, var681.iframeOrigin);
				}
			},
			getResponse: function(var684) {
				var var685;
				if (typeof var684 == "undefined") {
					var var686 = var617();
					if (var686) {
						var var687 = obj9.widgetMap.get(var686);
						var687 != null && var687.isExpired && fn24("Call to getResponse on a widget that expired, consider refreshing the widget.");
						return var687 == null ? undefined : var687.response;
					}
					fn23("Could not find a widget", 43794);
				}
				var var688 = pick20(var684);
				var688 || fn23("Could not find widget for provided container", 43778);
				return (var685 = obj9.widgetMap.get(var688)) === null || var685 === undefined ? undefined : var685.response;
			},
			isExpired: function(var689) {
				var var690, var691;
				if (typeof var689 == "undefined") {
					var var692 = var617();
					if (var692) {
						var var693, var694;
						return (var693 = (var694 = obj9.widgetMap.get(var692)) === null || var694 === undefined ? undefined : var694.isExpired) !== null && var693 !== undefined ? var693 : false;
					}
					fn23("Could not find a widget", 43794);
				}
				var var695 = pick20(var689);
				var695 || fn23("Could not find widget for provided container", 43778);
				return (var690 = (var691 = obj9.widgetMap.get(var695)) === null || var691 === undefined ? undefined : var691.isExpired) !== null && var690 !== undefined ? var690 : false;
			},
			ready: function(var696) {
				if (obj9.scriptWasLoadedAsync && (fn24("turnstile.ready() would break if called *before* the Turnstile api.js script is loaded by visitors."), fn23("Remove async/defer from the Turnstile api.js script tag before using turnstile.ready().", 3857)), typeof var696 != "function" && fn23("turnstile.ready() expected a \"function\" argument, got \"".concat(typeof var696 == "undefined" ? "undefined" : typeOf(var696), "\""), 3841), obj9.isReady) {
					var696();
					return;
				}
				arr7.push(var696);
			},
			remove: var551,
			render: call21,
			reset: var532
		});
		Object.defineProperty(var675, var365, {
			configurable: true,
			enumerable: false,
			value: {
				clearPendingApiJsReloadRequest: function() {
					fn49();
				},
				rearmTimedUpgrade: function() {
					fn53();
				},
				rejectPendingApiJsReloadRequest: function() {
					fn50();
				},
				reloadAfterUpgrade: function() {
					var555();
				}
			}
		});
		return {
			runImplicitRender: var615,
			turnstile: var675
		};
	})(), var697 = var470.runImplicitRender, var698 = var470.turnstile;
	function fn65(var699) {
		var var700, var701, obj16 = { sitekey: var699.getAttribute("data-sitekey") }, var702 = var699.getAttribute("data-tabindex");
		var702 && (obj16.tabindex = Number.parseInt(var702, 10));
		var var703 = var699.getAttribute("data-theme");
		var703 && (notEqual1(var703) ? obj16.theme = var703 : fn24("Unknown data-theme value: \"".concat(var703, "\".")));
		var var704 = var699.getAttribute("data-size");
		if (var704 && (notEqual3(var704) ? obj16.size = var704 : fn24("Unknown data-size value: \"".concat(var704, "\"."))), 0) var var705;
		var var706 = var699.getAttribute("data-action");
		typeof var706 == "string" && (obj16.action = var706);
		var var707 = var699.getAttribute("data-cdata");
		typeof var707 == "string" && (obj16.cData = var707);
		var var708 = var699.getAttribute("data-retry");
		var708 && (notEqual2(var708) ? obj16.retry = var708 : fn24("Invalid data-retry value: \"".concat(var708, ", expected either 'never' or 'auto'\".")));
		var var709 = var699.getAttribute("data-retry-interval");
		if (var709) {
			var var710 = Number.parseInt(var709, 10);
			var710 > 0 && var710 < 9e5 ? obj16["retry-interval"] = var710 : fn24("Invalid data-retry-interval value: \"".concat(var709, ", expected an integer value > 0 and < 900000\"."));
		}
		var var711 = var699.getAttribute("data-expiry-interval");
		if (var711) {
			var var712 = Number.parseInt(var711, 10);
			var712 > 0 && var712 < 36e4 ? obj16["expiry-interval"] = var712 : fn24("Invalid data-expiry-interval value: \"".concat(var712, ", expected an integer value > 0 and < 360000\"."));
		}
		var var713 = var699.getAttribute("data-refresh-expired");
		var713 && (notEqual4(var713) ? obj16["refresh-expired"] = var713 : fn24("Unknown data-refresh-expired value: \"".concat(var713, ", expected either: 'never', 'auto' or 'manual'.")));
		var var714 = var699.getAttribute("data-refresh-timeout");
		var714 && (notEqual5(var714) ? obj16["refresh-timeout"] = var714 : fn24("Unknown data-refresh-timeout value: \"".concat(var714, ", expected either: 'never', 'auto' or 'manual'.")));
		var var715 = var699.getAttribute("data-language");
		var715 && (or2(var715) ? obj16.language = var715 : fn24("Invalid data-language value: \"".concat(var715, ", expected either: auto, or an ISO 639-1 two-letter language code (e.g. en) or language and country code (e.g. en-US).")));
		function pick21(var716) {
			var var717 = var699.getAttribute(var716), var718 = var717 ? win[var717] : undefined;
			return typeof var718 == "function" ? var718 : undefined;
		}
		[
			"error-callback",
			"unsupported-callback",
			"callback",
			"expired-callback",
			"timeout-callback",
			"after-interactive-callback",
			"before-interactive-callback"
		].forEach(function(var719) {
			Object.assign(obj16, fn5({}, var719, pick21("data-".concat(var719))));
		});
		obj16["feedback-enabled"] = (var700 = pick18(var699.getAttribute("data-feedback-enabled"), true, function(var720) {
			return "Invalid data-feedback-enabled value: \"".concat(var720, "\", expected either: 'true' or 'false'. Value is ignored.");
		})) !== null && var700 !== undefined ? var700 : true;
		obj16["response-field"] = (var701 = pick18(var699.getAttribute("data-response-field"), true, function(var721) {
			return "Invalid data-response-field value: \"".concat(var721, "\", expected either: 'true' or 'false'. Value is ignored.");
		})) !== null && var701 !== undefined ? var701 : true;
		var var722 = var699.getAttribute("data-response-field-name");
		var722 && (obj16["response-field-name"] = var722);
		var var723 = var699.getAttribute("data-execution");
		var723 && (notEqual8(var723) ? obj16.execution = var723 : fn24("Unknown data-execution value: \"".concat(var723, ", expected either: 'render' or 'execute'.")));
		var var724 = var699.getAttribute("data-appearance");
		var724 && (notEqual6(var724) ? obj16.appearance = var724 : fn24("Unknown data-appearance value: \"".concat(var724, ", expected either: 'always', 'execute', or 'interaction-only'.")));
		var var725 = pick18(var699.getAttribute("data-offlabel-show-privacy"), undefined, function(var726) {
			return "Invalid data-offlabel-show-privacy value: \"".concat(var726, "\", expected \"true\" or \"false\".");
		});
		typeof var725 == "boolean" && (obj16["offlabel-show-privacy"] = var725);
		var var727 = pick18(var699.getAttribute("data-offlabel-show-help"), undefined, function(var728) {
			return "Invalid data-offlabel-show-help value: \"".concat(var728, "\", expected \"true\" or \"false\".");
		});
		typeof var727 == "boolean" && (obj16["offlabel-show-help"] = var727);
		return obj16;
	}
	function pick22() {
		if (fn47(), false) return false;
		return fn42(window.turnstile, obj9) ? true : (fn53(), false);
	}
	var734 = false;
	var735 = fn27();
	obj9.scriptWasLoadedAsync = (var732 = var735 == null ? undefined : var735.loadedAsync) !== null && var732 !== undefined ? var732 : false;
	obj9.scriptUrl = (var733 = var735 == null ? undefined : var735.src) !== null && var733 !== undefined ? var733 : "undefined";
	obj9.scriptUrlParsed = var735 == null ? undefined : var735.url;
	var735 != null && var735.params && (var736 = var735.params.get("compat"), (var736 == null ? undefined : var736.toLowerCase()) === "recaptcha" ? window.grecaptcha ? fn24("grecaptcha is already defined. The compatibility layer will not be enabled.") : (fn24("Compatibility layer enabled."), obj9.isRecaptchaCompatibilityMode = true, window.grecaptcha = var698) : var736 !== null && fn24("Unknown value for api.js?compat: \"".concat(var736, "\", ignoring.")), var735.params.forEach(function(var729, var730) {
		[
			"onload",
			"compat",
			"_cb",
			"_upgrade",
			"_reload",
			"render"
		].indexOf(var730) !== -1 || fn24("Unknown parameter passed to api.js: \"?".concat(var730, "=...\", ignoring."));
	}), var734 = var735.params.get("_upgrade") === "true", var737 = var735.params.get("onload"), var737 && !var734 && setTimeout(function() {
		typeof win[var737] == "function" ? win[var737]() : (fn24("Unable to find onload callback '".concat(var737, "' immediately after loading, expected 'function', got '").concat(typeOf(win[var737]), "'.")), setTimeout(function() {
			typeof win[var737] == "function" ? win[var737]() : fn24("Unable to find onload callback '".concat(var737, "' after 1 second, expected 'function', got '").concat(typeOf(win[var737]), "'."));
		}, 1e3));
	}, 0));
	var738 = "turnstile" in window;
	var739 = var738 ? fn45(window.turnstile) : undefined;
	var740 = var738 && var734 ? fn43(window.turnstile, obj9, function() {
		var var731;
		window.turnstile = var698;
		(var731 = fn45(var698)) === null || var731 === undefined || var731.reloadAfterUpgrade();
		fn37(obj9);
	}) : false;
	var740 && (var739 == null || var739.clearPendingApiJsReloadRequest());
	var738 && var734 && !var740 ? (fn24("Turnstile upgrade state was missing. Keeping the existing Turnstile instance."), var739 == null || var739.rejectPendingApiJsReloadRequest(), var739 == null || var739.rearmTimedUpgrade()) : var738 && !var734 ? fn24("Turnstile already has been loaded. Was Turnstile imported multiple times?") : (var740 || (window.turnstile = var698), var734 || ((var735 == null || (var741 = var735.params) === null || var741 === undefined ? undefined : var741.get("render")) !== "explicit" && arr7.push(var697), document.readyState === "complete" || document.readyState === "interactive" ? setTimeout(fn55, 0) : window.addEventListener("DOMContentLoaded", fn55)), fn53());
	var var732, var733, var734, var735, var736, var737, var738, var739, var740, var741;
})();
