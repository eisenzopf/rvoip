// Populate the sidebar
//
// This is a script, and not included directly in the page, to control the total size of the book.
// The TOC contains an entry for each page, so if each page includes a copy of the TOC,
// the total size of the page becomes O(n**2).
class MDBookSidebarScrollbox extends HTMLElement {
    constructor() {
        super();
    }
    connectedCallback() {
        this.innerHTML = '<ol class="chapter"><li class="chapter-item expanded affix "><a href="index.html">Introduction</a></li><li class="chapter-item expanded "><a href="part1/tutorial_01.html"><strong aria-hidden="true">1.</strong> Introduction to SIP</a></li><li class="chapter-item expanded "><a href="part1/tutorial_02.html"><strong aria-hidden="true">2.</strong> Parsing Your First SIP Message</a></li><li class="chapter-item expanded "><a href="part1/tutorial_03.html"><strong aria-hidden="true">3.</strong> Creating SIP Messages with the Builder Pattern</a></li><li class="chapter-item expanded "><a href="part1/tutorial_04.html"><strong aria-hidden="true">4.</strong> SIP Requests in Depth</a></li><li class="chapter-item expanded "><a href="part1/tutorial_05.html"><strong aria-hidden="true">5.</strong> SIP Responses in Depth</a></li><li class="chapter-item expanded "><a href="part2/tutorial_06.html"><strong aria-hidden="true">6.</strong> Introduction to SDP</a></li><li class="chapter-item expanded "><a href="part2/tutorial_07.html"><strong aria-hidden="true">7.</strong> Creating SDP Messages</a></li><li class="chapter-item expanded "><a href="part2/tutorial_08.html"><strong aria-hidden="true">8.</strong> Integrating SDP with SIP</a></li><li class="chapter-item expanded "><a href="part2/tutorial_09.html"><strong aria-hidden="true">9.</strong> Media Negotiation with SDP</a></li><li class="chapter-item expanded "><a href="part3/tutorial_10.html"><strong aria-hidden="true">10.</strong> SIP Transactions</a></li><li class="chapter-item expanded "><a href="part3/tutorial_11.html"><strong aria-hidden="true">11.</strong> SIP Dialogs</a></li><li class="chapter-item expanded "><a href="part3/tutorial_12.html"><strong aria-hidden="true">12.</strong> Complete Call Flow</a></li><li class="chapter-item expanded "><a href="part4/tutorial_13.html"><strong aria-hidden="true">13.</strong> Authentication</a></li><li class="chapter-item expanded "><a href="part4/tutorial_14.html"><strong aria-hidden="true">14.</strong> SIP Registration</a></li><li class="chapter-item expanded "><a href="part4/tutorial_15.html"><strong aria-hidden="true">15.</strong> SIP Proxying and Routing</a></li><li class="chapter-item expanded "><a href="part4/tutorial_16.html"><strong aria-hidden="true">16.</strong> Event Notification Framework</a></li><li class="chapter-item expanded "><a href="part5/tutorial_17.html"><strong aria-hidden="true">17.</strong> Building a SIP Client</a></li><li class="chapter-item expanded "><a href="part5/tutorial_18.html"><strong aria-hidden="true">18.</strong> WebRTC Integration</a></li><li class="chapter-item expanded "><a href="part5/tutorial_19.html"><strong aria-hidden="true">19.</strong> SIP Troubleshooting</a></li><li class="chapter-item expanded "><a href="part5/tutorial_20.html"><strong aria-hidden="true">20.</strong> Advanced Use Cases</a></li><li class="chapter-item expanded affix "><li class="spacer"></li><li class="chapter-item expanded affix "><a href="appendix/rfc_references.html">Appendix: SIP RFCs</a></li><li class="chapter-item expanded affix "><a href="appendix/glossary.html">Glossary</a></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString().split("#")[0].split("?")[0];
        if (current_page.endsWith("/")) {
            current_page += "index.html";
        }
        var links = Array.prototype.slice.call(this.querySelectorAll("a"));
        var l = links.length;
        for (var i = 0; i < l; ++i) {
            var link = links[i];
            var href = link.getAttribute("href");
            if (href && !href.startsWith("#") && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The "index" page is supposed to alias the first chapter in the book.
            if (link.href === current_page || (i === 0 && path_to_root === "" && current_page.endsWith("/index.html"))) {
                link.classList.add("active");
                var parent = link.parentElement;
                if (parent && parent.classList.contains("chapter-item")) {
                    parent.classList.add("expanded");
                }
                while (parent) {
                    if (parent.tagName === "LI" && parent.previousElementSibling) {
                        if (parent.previousElementSibling.classList.contains("chapter-item")) {
                            parent.previousElementSibling.classList.add("expanded");
                        }
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', function(e) {
            if (e.target.tagName === 'A') {
                sessionStorage.setItem('sidebar-scroll', this.scrollTop);
            }
        }, { passive: true });
        var sidebarScrollTop = sessionStorage.getItem('sidebar-scroll');
        sessionStorage.removeItem('sidebar-scroll');
        if (sidebarScrollTop) {
            // preserve sidebar scroll position when navigating via links within sidebar
            this.scrollTop = sidebarScrollTop;
        } else {
            // scroll sidebar to current active section when navigating via "next/previous chapter" buttons
            var activeSection = document.querySelector('#sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        var sidebarAnchorToggles = document.querySelectorAll('#sidebar a.toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(function (el) {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define("mdbook-sidebar-scrollbox", MDBookSidebarScrollbox);
