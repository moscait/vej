package com.moscait.vej.demo.service;

import com.moscait.vej.demo.domain.CallCategory;
import com.moscait.vej.demo.domain.CallExample;
import java.util.List;

/**
 * Curated set of 25 creative, realistic customer service interactions (5 per category).
 *
 * <p>Each example contains realistic multi-sentence context—such as agent greetings, caller
 * pleasantries, or follow-up details—so the demo can highlight how extracting the first
 * reason-for-call sentence preserves categorization accuracy while cutting input tokens.
 */
public final class CustomerServiceExamples {

    private static final List<CallExample> EXAMPLES = List.of(
            // --- COMPLAINT (5) ---
            new CallExample(
                    1,
                    "CS-01",
                    CallCategory.COMPLAINT,
                    "Phone",
                    "Diane Mercer",
                    """
                    Agent: Welcome to Apex Courier support, how can I help you today?
                    Customer: Hi there. I'm calling to file a formal complaint about the driver who dumped our refrigerated medical supplies in a puddle and forged my signature on the tablet. Nobody even rang the clinic bell, and the outer insulation box is completely waterlogged.
                    """),
            new CallExample(
                    2,
                    "CS-02",
                    CallCategory.COMPLAINT,
                    "Phone",
                    "Marcus Vance",
                    """
                    Good morning, thanks for picking up. I waited home for four hours today and your field technician failed to show up for the second appointment in a row without a single text or call. I had to burn another half-day of PTO, and I want this escalated to your regional dispatch manager.
                    """),
            new CallExample(
                    3,
                    "CS-03",
                    CallCategory.COMPLAINT,
                    "Chat",
                    "Nadia Cho",
                    """
                    Hello! I want to report how rude and dismissive the representative on my previous call was before disconnecting on me mid-sentence. When I asked why my commercial account stayed locked after submitting the paperwork, she laughed and said I should read the FAQ faster.
                    """),
            new CallExample(
                    4,
                    "CS-04",
                    CallCategory.COMPLAINT,
                    "Email",
                    "Tariq Al-Mansoor",
                    """
                    Hi team, hope you're doing well. I'm writing to complain about the unacceptable quality of the oak dining table delivered yesterday—two corner legs are deeply gouged and the hardware pouch was torn open inside the crate. This is the third consecutive shipment from your warehouse that arrived damaged.
                    """),
            new CallExample(
                    5,
                    "CS-05",
                    CallCategory.COMPLAINT,
                    "Phone",
                    "Rachel Sterling",
                    """
                    Agent: Billing and accounts, this is Jordan speaking.
                    Customer: Hi Jordan, how are you doing? I'm extremely frustrated because your automated system slapped a $35 late fee and sent a collections warning even though our autopay draft cleared three days before the due date. We have been loyal clients for six years and expect this billing breakdown reviewed immediately.
                    """),

            // --- REFUND (5) ---
            new CallExample(
                    6,
                    "CS-06",
                    CallCategory.REFUND,
                    "Phone",
                    "Carlos Mendez",
                    """
                    Agent: Thank you for calling Northstar Retail, my name is Sam.
                    Customer: Hi Sam. I was double-charged $149.99 on my Visa for order #NS-88412 this morning and need the duplicate charge refunded to my card. Your checkout spinner froze after I pressed pay once, and then two identical authorizations hit my bank statement.
                    """),
            new CallExample(
                    7,
                    "CS-07",
                    CallCategory.REFUND,
                    "Chat",
                    "Hannah Lindqvist",
                    """
                    Hi there! My tracking shows the winter parka I returned arrived back at your Reno warehouse twelve days ago, so I'm reaching out to get my $218.50 refund issued. I've attached the carrier drop-off receipt in case your returns desk needs the RMA barcode.
                    """),
            new CallExample(
                    8,
                    "CS-08",
                    CallCategory.REFUND,
                    "Phone",
                    "Devon Brooks",
                    """
                    Hi, my name is Devon. The studio headphones delivered yesterday have a dead left earcup right out of the box, so I'd like to return them for a full refund rather than waiting for an exchange. I still have all the original foam inserts ready to drop at UPS.
                    """),
            new CallExample(
                    9,
                    "CS-09",
                    CallCategory.REFUND,
                    "Ticket",
                    "Priya Nair",
                    """
                    Good afternoon, thank you for your time. My annual membership renewed at the full $180 rate instead of applying my verified employer discount, so please reimburse the $60 difference back to my payment method. Let me know if you need the verification confirmation code again.
                    """),
            new CallExample(
                    10,
                    "CS-10",
                    CallCategory.REFUND,
                    "Phone",
                    "Owen Gallagher",
                    """
                    Agent: Harbor Transit reservations, Elena speaking.
                    Customer: Hey Elena, thanks for picking up. Our pre-paid airport shuttle never arrived at Terminal B at 6:15 a.m. and we had to take a cab, so I'm calling to request a full refund for booking #HT-4091. I have the receipt from the taxi too if your claims desk covers alternate transport.
                    """),

            // --- SERVICE CANCELLATION (5) ---
            new CallExample(
                    11,
                    "CS-11",
                    CallCategory.SERVICE_CANCELLATION,
                    "Phone",
                    "Sofia Rossi",
                    """
                    Agent: CloudVault account care, this is Leo.
                    Customer: Good afternoon, Leo, hope your week is going well. I need to cancel my personal CloudVault Pro subscription before next Tuesday's renewal date. My company moved everyone onto an enterprise workspace, so I won't need the individual plan anymore.
                    """),
            new CallExample(
                    12,
                    "CS-12",
                    CallCategory.SERVICE_CANCELLATION,
                    "Chat",
                    "Noah Takahashi",
                    """
                    Hi! We're relocating overseas next month outside your service footprint, so please terminate our home fiber internet plan effective the 30th. Could you also confirm where to drop off the optical gateway router so we aren't billed an unreturned equipment fee?
                    """),
            new CallExample(
                    13,
                    "CS-13",
                    CallCategory.SERVICE_CANCELLATION,
                    "Phone",
                    "Chloe Bennett",
                    """
                    Hello, thanks for taking my call. I'd like to permanently close my weekly meal-kit membership today rather than pausing it again. Our kids started college this fall and we just aren't cooking enough at home to justify the recurring box.
                    """),
            new CallExample(
                    14,
                    "CS-14",
                    CallCategory.SERVICE_CANCELLATION,
                    "Phone",
                    "Benny Kowalski",
                    """
                    Agent: ClearLine Business Desk, how may I assist you?
                    Customer: Hi there, this is Benny. Our seasonal pop-up shop wrapped up yesterday, so I'm calling to disconnect our two temporary VoIP business lines and stop all future billing. Please keep our archived call logs accessible for tax season if possible.
                    """),
            new CallExample(
                    15,
                    "CS-15",
                    CallCategory.SERVICE_CANCELLATION,
                    "Chat",
                    "Amina Diallo",
                    """
                    Good morning. We decided to bundle our auto coverage with another insurer starting Friday, so please process a cancellation for policy #AUTO-77310 effective at 12:01 a.m. on the 18th. Let me know where I should upload the binder from our new carrier.
                    """),

            // --- SUPPORT (5) ---
            new CallExample(
                    16,
                    "CS-16",
                    CallCategory.SUPPORT,
                    "Phone",
                    "Liam O'Connor",
                    """
                    Agent: Beacon Cloud technical desk, Maya speaking.
                    Customer: Yeah, hi Maya. I need help resetting MFA and restoring backup recovery codes because my authenticator app got wiped when I replaced my phone yesterday. Right now none of our team admins can approve the pending SSO rotation.
                    """),
            new CallExample(
                    17,
                    "CS-17",
                    CallCategory.SUPPORT,
                    "Phone",
                    "Grace Kim",
                    """
                    Agent: Northstar Fiber technical support, this is Andre.
                    Customer: Hi Andre, hope your morning is going well! I'm calling for troubleshooting help because our office router has been dropping Wi-Fi every twenty minutes since last night's firmware update. I already power-cycled the modem twice and verified the fiber light stays solid green.
                    """),
            new CallExample(
                    18,
                    "CS-18",
                    CallCategory.SUPPORT,
                    "Chat",
                    "Victor Morales",
                    """
                    Hi! I just unboxed our new smart thermostat and need assistance pairing it with a dual-band mesh network that combines 2.4 GHz and 5 GHz under one SSID. The mobile app keeps timing out at step three right after I enter the home Wi-Fi password.
                    """),
            new CallExample(
                    19,
                    "CS-19",
                    CallCategory.SUPPORT,
                    "Ticket",
                    "Sienna Park",
                    """
                    Hello support team. We're onboarding twelve new hires on Monday and need step-by-step guidance configuring SAML single sign-on with Okta and provisioning role permissions. Could you send the updated metadata XML endpoint and confirm whether group sync runs automatically?
                    """),
            new CallExample(
                    20,
                    "CS-20",
                    CallCategory.SUPPORT,
                    "Phone",
                    "Mateo Alvarez",
                    """
                    Agent: Summit Logistics support, how can I help?
                    Customer: Good afternoon. My shipment #SL-99204 has been sitting at the Memphis sorting hub for four days without a scan, and I need help checking the carrier status and adding gate access instructions. The recipient works from home and just wants to make sure the courier knows the keypad code.
                    """),

            // --- POSITIVE FEEDBACK (5) ---
            new CallExample(
                    21,
                    "CS-21",
                    CallCategory.POSITIVE_FEEDBACK,
                    "Phone",
                    "Elena Vance",
                    """
                    Agent: PayrollFlow support desk, Alex speaking.
                    Customer: Hey Alex, hope your week is going well! I actually called in just to pass along glowing praise for Priya on your onboarding team, who stayed past closing yesterday to make sure our year-end tax import succeeded. We rarely get support that thorough and wanted her manager to know.
                    """),
            new CallExample(
                    22,
                    "CS-22",
                    CallCategory.POSITIVE_FEEDBACK,
                    "Chat",
                    "Julian Scott",
                    """
                    Hi there! I just wanted to drop a quick thank-you note for how transparently your engineering team handled last night's database maintenance and kept the status page updated every ten minutes. Zero customer orders were lost on our storefront and our whole team noticed the extra care.
                    """),
            new CallExample(
                    23,
                    "CS-23",
                    CallCategory.POSITIVE_FEEDBACK,
                    "Phone",
                    "Nora Jenkins",
                    """
                    Agent: GridPoint Energy customer care, this is Tessa.
                    Customer: Good morning, Tessa! Nothing is broken today—I'm calling solely to commend your technician Marcus, who went above and beyond labeling our 1970s breaker panel cleanly after finishing the solar inverter install. He left the garage spotless and patiently walked us through the monitoring app.
                    """),
            new CallExample(
                    24,
                    "CS-24",
                    CallCategory.POSITIVE_FEEDBACK,
                    "Email",
                    "Isaac Chen",
                    """
                    Hello! I wanted to share enthusiastic feedback on the redesigned mobile expense app—the instant receipt OCR and automated split categories cut our Friday bookkeeping time in half. Thanks for listening to user requests and keeping the interface so snappy!
                    """),
            new CallExample(
                    25,
                    "CS-25",
                    CallCategory.POSITIVE_FEEDBACK,
                    "Phone",
                    "Miriam Patel",
                    """
                    Agent: TrailPeak Gear warranty line, Devon speaking.
                    Customer: Hi Devon, thank you for picking up. I'm calling to thank your warranty team for overnighting a replacement tent pole set before our Yosemite trip after mine snapped in high wind. That kind of customer care turned our whole hiking group into lifelong customers.
                    """)
    );

    private CustomerServiceExamples() {}

    public static List<CallExample> all() {
        return EXAMPLES;
    }
}
